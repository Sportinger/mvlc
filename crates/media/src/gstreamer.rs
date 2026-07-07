//! Minimal FFmpeg media backend for environments without the native GStreamer SDK.

use crate::audio::AudioSampleSink;
use mvlc_core::StreamId;
use std::collections::HashMap;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use tracing::{debug, info};

pub fn init() -> Result<(), Box<dyn std::error::Error>> {
    Command::new("ffmpeg").arg("-version").output()?;
    Ok(())
}

pub fn check_vaapi_support() -> bool {
    false
}

pub fn check_dmabuf_support() -> bool {
    false
}

#[derive(Debug, Clone)]
pub struct DmaBufInfo {
    pub fd: i32,
    pub size: usize,
}

#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub stream_id: StreamId,
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub pts: u64,
    pub data: Vec<u8>,
    pub is_dmabuf: bool,
    pub colorimetry: Option<VideoColorimetry>,
}

#[derive(Debug, Clone)]
pub struct VideoColorimetry {
    pub primaries: String,
    pub transfer: String,
    pub matrix: String,
    pub max_cll: Option<f32>,
    pub max_fall: Option<f32>,
}

impl VideoFrame {
    pub fn extract_dmabuf_fds(&self) -> Option<Vec<DmaBufInfo>> {
        None
    }

    pub fn data_size_bytes(&self) -> usize {
        self.data.len()
    }
}

pub struct HardwareVideoDecoder {
    frame_receiver: Receiver<VideoFrame>,
    child: Arc<Mutex<Option<Child>>>,
    playing: Arc<AtomicBool>,
    position_ns: Arc<AtomicU64>,
    duration_ns: Option<u64>,
}

impl HardwareVideoDecoder {
    pub fn new(
        stream_id: StreamId,
        file_path: &str,
        _audio_sink: Option<AudioSampleSink>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let probe = probe_video(file_path)?;
        let (width, height) = scaled_dimensions(probe.width, probe.height);
        let fps = probe.fps.min(30).max(1);
        let frame_size = width as usize * height as usize * 4;
        let (frame_sender, frame_receiver) = mpsc::sync_channel(2);

        let vf = format!("fps={fps},scale={width}:{height}:flags=fast_bilinear,format=rgba");
        let mut child = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-re",
                "-i",
                file_path,
                "-an",
                "-vf",
                &vf,
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let stdout = child.stdout.take().ok_or("ffmpeg stdout unavailable")?;
        let child = Arc::new(Mutex::new(Some(child)));
        let playing = Arc::new(AtomicBool::new(false));
        let position_ns = Arc::new(AtomicU64::new(0));

        spawn_reader(
            stream_id,
            stdout,
            frame_sender,
            width,
            height,
            frame_size,
            fps,
            Arc::clone(&playing),
            Arc::clone(&position_ns),
            Arc::clone(&child),
        );

        info!(
            "FFmpeg decoder stream {}: {}x{} -> {}x{} @ {}fps",
            stream_id.0, probe.width, probe.height, width, height, fps
        );

        Ok(Self {
            frame_receiver,
            child,
            playing,
            position_ns,
            duration_ns: probe.duration_ns,
        })
    }

    pub fn play(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.playing.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub fn pause(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.playing.store(false, Ordering::Relaxed);
        Ok(())
    }

    pub fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.playing.store(false, Ordering::Relaxed);
        self.position_ns.store(0, Ordering::Relaxed);
        Ok(())
    }

    pub fn seek(&self, position_ns: u64) -> Result<(), Box<dyn std::error::Error>> {
        self.position_ns.store(position_ns, Ordering::Relaxed);
        Ok(())
    }

    pub fn try_recv_frame(&self) -> Option<VideoFrame> {
        self.frame_receiver.try_recv().ok()
    }

    pub fn duration(&self) -> Option<u64> {
        self.duration_ns
    }

    pub fn position(&self) -> Option<u64> {
        Some(self.position_ns.load(Ordering::Relaxed))
    }

    pub fn is_hardware_accelerated(&self) -> bool {
        false
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }
}

impl Drop for HardwareVideoDecoder {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            if let Some(mut child) = child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

struct VideoProbe {
    width: u32,
    height: u32,
    fps: u64,
    duration_ns: Option<u64>,
}

fn probe_video(file_path: &str) -> Result<VideoProbe, Box<dyn std::error::Error>> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,r_frame_rate,duration",
            "-of",
            "default=nokey=1:noprint_wrappers=1",
            file_path,
        ])
        .output()?;

    if !output.status.success() {
        return Err(format!("ffprobe failed for {file_path}").into());
    }

    let stdout = String::from_utf8(output.stdout)?;
    let mut lines = stdout.lines();
    let width = lines.next().ok_or("ffprobe missing width")?.parse::<u32>()?;
    let height = lines
        .next()
        .ok_or("ffprobe missing height")?
        .parse::<u32>()?;
    let fps = lines.next().and_then(parse_fps).unwrap_or(30);
    let duration_ns = lines
        .next()
        .and_then(|line| line.parse::<f64>().ok())
        .map(|seconds| (seconds * 1_000_000_000.0) as u64);

    Ok(VideoProbe {
        width,
        height,
        fps,
        duration_ns,
    })
}

fn parse_fps(value: &str) -> Option<u64> {
    let (num, den) = value.split_once('/')?;
    let num = num.parse::<f64>().ok()?;
    let den = den.parse::<f64>().ok()?;
    if den == 0.0 {
        return None;
    }
    Some((num / den).round() as u64)
}

fn scaled_dimensions(width: u32, height: u32) -> (u32, u32) {
    let max_width = std::env::var("MVLC_MAX_DECODE_WIDTH")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(640);

    if width <= max_width {
        return (width.max(2) & !1, height.max(2) & !1);
    }

    let scaled_height = ((height as u64 * max_width as u64 / width as u64) as u32).max(2) & !1;
    (max_width.max(2) & !1, scaled_height)
}

#[allow(clippy::too_many_arguments)]
fn spawn_reader(
    stream_id: StreamId,
    mut stdout: impl Read + Send + 'static,
    frame_sender: mpsc::SyncSender<VideoFrame>,
    width: u32,
    height: u32,
    frame_size: usize,
    fps: u64,
    playing: Arc<AtomicBool>,
    position_ns: Arc<AtomicU64>,
    child: Arc<Mutex<Option<Child>>>,
) {
    thread::spawn(move || {
        let mut frame_no = 0u64;
        loop {
            let mut data = vec![0u8; frame_size];
            if stdout.read_exact(&mut data).is_err() {
                break;
            }

            if !playing.load(Ordering::Relaxed) {
                frame_no += 1;
                continue;
            }

            let pts = frame_no * 1_000_000_000 / fps;
            position_ns.store(pts, Ordering::Relaxed);
            frame_no += 1;

            let frame = VideoFrame {
                stream_id,
                width,
                height,
                format: "RGBA".to_string(),
                pts,
                data,
                is_dmabuf: false,
                colorimetry: None,
            };

            match frame_sender.try_send(frame) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => debug!("Dropped stale frame for stream {}", stream_id.0),
                Err(TrySendError::Disconnected(_)) => break,
            }
        }

        if let Ok(mut child) = child.lock() {
            if let Some(mut child) = child.take() {
                let _ = child.wait();
            }
        }
    });
}

pub struct MediaManager {
    decoders: HashMap<StreamId, HardwareVideoDecoder>,
}

impl MediaManager {
    pub fn new() -> Self {
        Self {
            decoders: HashMap::new(),
        }
    }

    pub fn load_video(
        &mut self,
        stream_id: StreamId,
        file_path: &str,
        audio_sink: Option<AudioSampleSink>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let decoder = HardwareVideoDecoder::new(stream_id, file_path, audio_sink)?;
        self.decoders.insert(stream_id, decoder);
        info!("Loaded FFmpeg media stream {}", stream_id.0);
        Ok(())
    }

    pub fn unload_video(&mut self, stream_id: &StreamId) {
        self.decoders.remove(stream_id);
    }

    pub fn get_decoder(&self, stream_id: &StreamId) -> Option<&HardwareVideoDecoder> {
        self.decoders.get(stream_id)
    }

    pub fn active_streams(&self) -> Vec<StreamId> {
        self.decoders.keys().copied().collect()
    }

    pub fn hardware_status(&self) -> (usize, usize) {
        (0, self.decoders.len())
    }
}
