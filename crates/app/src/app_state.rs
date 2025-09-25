use std::collections::{HashMap, HashSet};
use std::time::Duration;
use winit::window::Window;

use crate::badges::{BadgeState, RuntimeBadge};
use crate::canvas::{CanvasInteraction, CanvasViewport};
use crate::transport::TransportState;
use crate::video_layer::VideoLayerState;
use egui_wgpu::Renderer as EguiWgpuRenderer;
use mvlc_core::{AvSyncManager, LayerId, PerformanceMonitor, Project, StreamId};
use mvlc_media::{AudioMasterClock, AudioOutput};
use mvlc_render::{ColorPipeline, Renderer};

pub struct AppState {
    pub project: Project,
    pub badges: Vec<RuntimeBadge>,
    pub audio_output: Option<AudioOutput>,
    pub canvas_viewport: CanvasViewport,
    pub canvas_interaction: CanvasInteraction,
    pub av_sync_manager: AvSyncManager<AudioMasterClock>,
    pub renderer: Renderer,
    pub performance_monitor: PerformanceMonitor,
    pub color_pipeline: Option<ColorPipeline>,
    pub transport: TransportState,
    pub(crate) video_layers: HashMap<LayerId, VideoLayerState>,
    layers_pending_fit: HashSet<LayerId>,
}

impl AppState {
    pub fn new(window: &Window) -> Self {
        let project = Project::new("Untitled Project".to_string(), 1920, 1080);

        let badges = vec![
            RuntimeBadge::new("Decode".to_string(), "SW".to_string(), BadgeState::Fallback),
            RuntimeBadge::new(
                "Transfer".to_string(),
                "Staged".to_string(),
                BadgeState::Partial,
            ),
            RuntimeBadge::new(
                "Performance".to_string(),
                "0 B/frame".to_string(),
                BadgeState::Fallback,
            ),
            RuntimeBadge::new(
                "Color".to_string(),
                "Basic".to_string(),
                BadgeState::Fallback,
            ),
            RuntimeBadge::new(
                "Render".to_string(),
                "wgpu".to_string(),
                BadgeState::Partial,
            ),
            RuntimeBadge::new(
                "Sync".to_string(),
                "A/V ±0ms".to_string(),
                BadgeState::Optimal,
            ),
        ];

        let audio_output = match AudioOutput::new_default() {
            Ok(audio) => {
                tracing::info!("Audio output initialized successfully");
                Some(audio)
            }
            Err(e) => {
                tracing::error!("Failed to initialize audio output: {}", e);
                None
            }
        };

        let mut av_sync_manager = AvSyncManager::new();

        if let Some(ref audio) = audio_output {
            for layer in project.layers.layers() {
                let stream_id = StreamId(layer.id.0 as u64);
                av_sync_manager.add_stream(stream_id, audio.master_clock().clone(), 30.0);
            }
        }

        let use_vulkan = false;

        let mut renderer = if use_vulkan {
            match Renderer::new_vulkan(window) {
                Ok(r) => {
                    tracing::info!("Using Vulkan renderer with DMA-BUF support");
                    r
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to create Vulkan renderer: {}, falling back to placeholder",
                        e
                    );
                    Renderer::default()
                }
            }
        } else {
            Renderer::default()
        };

        if let Err(e) = renderer.init() {
            tracing::warn!("Failed to initialize renderer: {}", e);
        }

        let color_pipeline = match ColorPipeline::new() {
            Ok(pipeline) => {
                tracing::info!("Color pipeline initialized successfully");
                Some(pipeline)
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to initialize color pipeline: {}, color management will be unavailable",
                    e
                );
                None
            }
        };

        Self {
            project,
            badges,
            audio_output,
            canvas_viewport: CanvasViewport::default(),
            canvas_interaction: CanvasInteraction {
                selected_layer: None,
                is_dragging: false,
                drag_start: [0.0, 0.0],
                gizmo_handle: None,
            },
            av_sync_manager,
            renderer,
            performance_monitor: PerformanceMonitor::new(),
            color_pipeline,
            transport: TransportState::default(),
            video_layers: HashMap::new(),
            layers_pending_fit: HashSet::new(),
        }
    }

    /// Poll all active video decoders for fresh frames and update performance stats
    pub fn poll_video_frames(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut EguiWgpuRenderer,
    ) {
        let mut fit_updates = Vec::new();

        for (layer_id, state) in self.video_layers.iter_mut() {
            if let Some(frame) = state.update_with_latest_frame(device, queue, renderer) {
                fit_updates.push((*layer_id, frame.width));
                let upload_bytes = frame.data_size_bytes() as u64;
                self.performance_monitor.record_frame(
                    frame.stream_id.0,
                    upload_bytes,
                    Duration::from_millis(16),
                    false,
                );
            }
        }

        for (layer_id, width) in fit_updates {
            self.apply_pending_fit(layer_id, width);
        }

        self.update_transport_from_decoders();
    }

    /// Drain video decoders without uploading frames. Useful for headless render paths.
    pub fn poll_video_frames_headless(&mut self) {
        let mut fit_updates = Vec::new();

        for (layer_id, state) in self.video_layers.iter_mut() {
            if let Some(frame) = state.drain_latest_frame() {
                fit_updates.push((*layer_id, frame.width));
                self.performance_monitor.record_frame(
                    frame.stream_id.0,
                    0,
                    Duration::from_millis(16),
                    false,
                );
            }
        }

        for (layer_id, width) in fit_updates {
            self.apply_pending_fit(layer_id, width);
        }

        self.update_transport_from_decoders();
    }

    pub fn video_layers_mut(&mut self) -> &mut HashMap<LayerId, VideoLayerState> {
        &mut self.video_layers
    }

    pub fn schedule_fit_to_view(&mut self, layer_id: LayerId) {
        self.layers_pending_fit.insert(layer_id);
    }

    pub fn play_all(&mut self) {
        let mut started_any = false;

        for (layer_id, state) in self.video_layers.iter() {
            match state.play() {
                Ok(_) => started_any = true,
                Err(err) => tracing::error!("Failed to play layer {}: {}", layer_id.0, err),
            }
        }

        self.update_transport_from_decoders();

        if started_any {
            self.transport.is_playing = true;
        }
    }

    pub fn pause_all(&mut self) {
        for (layer_id, state) in self.video_layers.iter() {
            if let Err(err) = state.pause() {
                tracing::error!("Failed to pause layer {}: {}", layer_id.0, err);
            }
        }

        self.transport.is_playing = false;
        self.update_transport_from_decoders();
    }

    pub fn stop_all(&mut self) {
        for (layer_id, state) in self.video_layers.iter() {
            if let Err(err) = state.pause() {
                tracing::error!("Failed to pause layer {}: {}", layer_id.0, err);
            }

            if let Err(err) = state.seek(0) {
                tracing::error!("Failed to seek layer {} to start: {}", layer_id.0, err);
            }
        }

        if let Some(audio) = &self.audio_output {
            audio.clear_buffer();
        }

        self.transport.is_playing = false;
        self.transport.position_seconds = 0.0;
        self.update_transport_from_decoders();
        self.transport.position_seconds = 0.0;
    }

    pub fn seek_all(&mut self, position_seconds: f32) {
        let clamped_seconds = position_seconds.max(0.0);
        let position_ns = (clamped_seconds * 1_000_000_000.0f32) as u64;

        for (layer_id, state) in self.video_layers.iter() {
            if let Err(err) = state.seek(position_ns) {
                tracing::error!(
                    "Failed to seek layer {} to {:.3}s: {}",
                    layer_id.0,
                    clamped_seconds,
                    err
                );
            }
        }

        if let Some(audio) = &self.audio_output {
            audio.clear_buffer();
        }

        self.transport.position_seconds = clamped_seconds;
        self.update_transport_from_decoders();
    }

    pub fn update_transport_from_decoders(&mut self) {
        if self.video_layers.is_empty() {
            self.transport = TransportState::default();
            return;
        }

        let mut playing = false;
        let mut duration: Option<f32> = None;
        let mut position: Option<f32> = None;

        for state in self.video_layers.values() {
            if state.is_playing() {
                playing = true;
            }

            if let Some(layer_duration) = state.duration_seconds() {
                duration = Some(duration.map_or(layer_duration, |curr| curr.max(layer_duration)));
            }

            if let Some(layer_position) = state.position_seconds() {
                position = Some(position.map_or(layer_position, |curr| curr.max(layer_position)));
            }
        }

        self.transport.is_playing = playing;

        if let Some(duration_seconds) = duration {
            self.transport.duration_seconds = duration_seconds;
        }

        if let Some(position_seconds) = position {
            let clamp_max = self.transport.duration_seconds.max(0.0);
            self.transport.position_seconds = position_seconds.clamp(0.0, clamp_max);
        } else {
            let clamp_max = self.transport.duration_seconds.max(0.0);
            self.transport.position_seconds = self.transport.position_seconds.clamp(0.0, clamp_max);
        }
    }

    fn apply_pending_fit(&mut self, layer_id: LayerId, frame_width: u32) {
        if !self.layers_pending_fit.remove(&layer_id) {
            return;
        }

        if frame_width == 0 {
            self.layers_pending_fit.insert(layer_id);
            return;
        }

        let viewport_width = self.canvas_viewport.size[0].max(1.0);
        let zoom = self.canvas_viewport.zoom.max(0.001);
        let target_scale = (viewport_width / (frame_width as f32 * zoom)).max(0.01);

        if let Some(layer) = self.project.layers.get_layer_mut(layer_id) {
            layer.set_scale(target_scale, target_scale);
            layer.set_position(0.0, 0.0);
        }
    }
}
