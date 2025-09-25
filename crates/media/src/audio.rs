//! Audio output and master clock for MVLC
//!
//! Provides audio output using cpal and maintains the master clock
//! for A/V synchronization.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use mvlc_core::{Clock, Time};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tracing::{debug, error, info, warn};

/// Audio master clock that provides timing based on audio samples played
///
/// This serves as the master clock for A/V synchronization, ensuring
/// video frames are presented in sync with audio playback.
#[derive(Clone)]
pub struct AudioMasterClock {
    /// Total samples played since start
    samples_played: Arc<AtomicU64>,
    /// Sample rate in Hz
    sample_rate: u32,
    /// Start time for reference
    start_time: Time,
}

impl AudioMasterClock {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            samples_played: Arc::new(AtomicU64::new(0)),
            sample_rate,
            start_time: Time::ZERO, // Will be set when audio starts
        }
    }

    /// Get the number of samples played
    pub fn samples_played(&self) -> u64 {
        self.samples_played.load(Ordering::Relaxed)
    }

    /// Set the number of samples played
    pub fn set_samples_played(&mut self, samples: u64) {
        self.samples_played.store(samples, Ordering::Relaxed);
    }

    /// Increment the sample counter by the given amount
    pub fn add_samples(&self, samples: u64) {
        self.samples_played.fetch_add(samples, Ordering::Relaxed);
    }

    /// Set the reference start time
    pub fn set_start_time(&mut self, time: Time) {
        self.start_time = time;
    }
}

impl Clock for AudioMasterClock {
    fn now(&self) -> Time {
        let samples = self.samples_played();
        let seconds = samples as f64 / self.sample_rate as f64;
        self.start_time.add(Time::from_secs(seconds as i64))
    }
}

/// Audio output configuration
#[derive(Debug, Clone)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub buffer_size: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44100,
            channels: 2,
            buffer_size: 1024,
        }
    }
}

struct AudioShared {
    buffer: Mutex<VecDeque<f32>>,
    capacity: usize,
}

impl AudioShared {
    fn push_samples(&self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }

        let mut buffer = self.buffer.lock().expect("audio buffer mutex poisoned");

        if samples.len() >= self.capacity {
            buffer.clear();
            buffer.extend(samples[samples.len() - self.capacity..].iter().copied());
            return;
        }

        let required = samples.len();
        let available = self.capacity.saturating_sub(buffer.len());

        if required > available {
            let overflow = required - available;
            for _ in 0..overflow {
                buffer.pop_front();
            }
        }

        buffer.extend(samples.iter().copied());
    }

    fn pop_samples(&self, dest: &mut [f32]) {
        let mut buffer = self.buffer.lock().expect("audio buffer mutex poisoned");

        for sample in dest.iter_mut() {
            *sample = buffer.pop_front().unwrap_or(0.0);
        }
    }

    fn clear(&self) {
        let mut buffer = self.buffer.lock().expect("audio buffer mutex poisoned");
        buffer.clear();
    }
}

#[derive(Clone)]
pub struct AudioSampleSink {
    shared: Arc<AudioShared>,
    sample_rate: u32,
    channels: u16,
}

impl AudioSampleSink {
    pub fn push_samples(&self, samples: &[f32]) {
        self.shared.push_samples(samples);
    }

    pub fn clear(&self) {
        self.shared.clear();
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }
}

/// Audio output stream that generates silence
///
/// Initially outputs silence, but can be extended to play actual audio data.
pub struct AudioOutput {
    _stream: Stream, // Keep stream alive
    config: AudioConfig,
    master_clock: AudioMasterClock,
    shared: Arc<AudioShared>,
}

impl AudioOutput {
    /// Create a new audio output with the given configuration
    pub fn new(config: AudioConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("No default audio output device found")?;

        info!("Using audio device: {}", device.name()?);
        debug!("Audio config: {:?}", config);

        // Find a suitable output config
        let mut supported_configs = device.supported_output_configs()?;
        let supported_config = supported_configs
            .find(|c| {
                c.sample_format() == SampleFormat::F32
                    && c.channels() == config.channels
                    && c.min_sample_rate().0 <= config.sample_rate
                    && c.max_sample_rate().0 >= config.sample_rate
            })
            .ok_or("No suitable audio output config found")?;

        let stream_config: StreamConfig = supported_config
            .with_sample_rate(cpal::SampleRate(config.sample_rate))
            .into();

        let effective_config = AudioConfig {
            sample_rate: stream_config.sample_rate.0,
            channels: stream_config.channels,
            buffer_size: config.buffer_size,
        };

        let buffer_capacity =
            (effective_config.sample_rate as usize * effective_config.channels as usize * 4)
                .max(effective_config.channels as usize * 1024);

        let shared = Arc::new(AudioShared {
            buffer: Mutex::new(VecDeque::with_capacity(buffer_capacity)),
            capacity: buffer_capacity,
        });

        let master_clock = AudioMasterClock::new(effective_config.sample_rate);

        let clock_clone = master_clock.clone();
        let shared_for_callback = Arc::clone(&shared);
        let callback_channels = effective_config.channels as usize;

        let stream = device.build_output_stream(
            &stream_config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                shared_for_callback.pop_samples(data);
                let frames = data.len() as u64 / callback_channels as u64;
                clock_clone.add_samples(frames);
            },
            move |err| {
                error!("Audio stream error: {}", err);
            },
            None,
        )?;

        stream.play()?;

        info!(
            "Audio output initialized with {} Hz, {} channels",
            effective_config.sample_rate, effective_config.channels
        );

        Ok(Self {
            _stream: stream,
            config: effective_config,
            master_clock,
            shared,
        })
    }

    /// Create audio output with default configuration
    pub fn new_default() -> Result<Self, Box<dyn std::error::Error>> {
        Self::new(AudioConfig::default())
    }

    /// Get the master clock
    pub fn master_clock(&self) -> &AudioMasterClock {
        &self.master_clock
    }

    /// Get the current audio configuration
    pub fn config(&self) -> &AudioConfig {
        &self.config
    }

    /// Get the current playback time according to the master clock
    pub fn current_time(&self) -> Time {
        self.master_clock.now()
    }

    /// Obtain a handle that allows pushing decoded PCM samples into the output queue
    pub fn sample_sink(&self) -> AudioSampleSink {
        AudioSampleSink {
            shared: Arc::clone(&self.shared),
            sample_rate: self.config.sample_rate,
            channels: self.config.channels,
        }
    }

    /// Enqueue interleaved PCM samples for playback
    pub fn enqueue_samples(&self, samples: &[f32]) {
        self.shared.push_samples(samples);
    }

    /// Clear any queued audio data (useful when seeking)
    pub fn clear_buffer(&self) {
        self.shared.clear();
    }
}

/// Audio resampler for converting between different sample rates
///
/// Currently a placeholder - full implementation would use a proper
/// resampling library like rubato or soxr.
pub struct AudioResampler {
    input_rate: u32,
    output_rate: u32,
}

impl AudioResampler {
    pub fn new(input_rate: u32, output_rate: u32) -> Self {
        Self {
            input_rate,
            output_rate,
        }
    }

    /// Resample audio data (placeholder implementation)
    pub fn resample(&self, input: &[f32]) -> Vec<f32> {
        // TODO: Implement proper resampling
        // For now, just return input unchanged
        warn!("Audio resampling not yet implemented - returning input unchanged");
        input.to_vec()
    }
}

/// Audio mixer for combining multiple audio sources
///
/// Placeholder for future multi-track audio mixing.
pub struct AudioMixer {
    sample_rate: u32,
    channels: u16,
}

impl AudioMixer {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            sample_rate,
            channels,
        }
    }

    /// Mix multiple audio streams (placeholder)
    pub fn mix(&self, sources: &[&[f32]]) -> Vec<f32> {
        if sources.is_empty() {
            return Vec::new();
        }

        let mut output = sources[0].to_vec();

        // Simple mixing by addition (would need normalization in real implementation)
        for source in &sources[1..] {
            for (i, &sample) in source.iter().enumerate() {
                if i < output.len() {
                    output[i] += sample;
                }
            }
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_audio_master_clock() {
        let mut clock = AudioMasterClock::new(44100);
        clock.set_start_time(Time::ZERO);

        // Initially zero
        assert_eq!(clock.samples_played(), 0);
        assert_eq!(clock.now(), Time::ZERO);

        // Add some samples
        clock.add_samples(44100); // 1 second at 44.1kHz
        assert_eq!(clock.samples_played(), 44100);
        assert_eq!(clock.now(), Time::from_secs(1));
    }

    #[test]
    fn test_audio_config_default() {
        let config = AudioConfig::default();
        assert_eq!(config.sample_rate, 44100);
        assert_eq!(config.channels, 2);
        assert_eq!(config.buffer_size, 1024);
    }

    // Note: AudioOutput tests would require actual audio hardware
    // and are difficult to test in CI. Integration tests would be better.
}
