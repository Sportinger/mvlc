//! A/V Synchronization for MVLC
//!
//! Implements drop/repeat window algorithm for maintaining audio/video synchronization.
//! Uses audio master clock as reference and adjusts video frame presentation timing.

use crate::{Clock, FrameTiming, Time};
use std::collections::VecDeque;

/// A/V synchronization state for a video stream
#[derive(Debug)]
pub struct AvSyncState {
    /// Target frame duration (1/fps)
    pub frame_duration: Time,
    /// Maximum allowed drift before correction (in milliseconds)
    pub max_drift_ms: f64,
    /// Number of frames in the correction window
    pub correction_window: usize,
    /// Current drift accumulator
    pub drift_accumulator: f64,
    /// Recent frame timing history
    pub frame_history: VecDeque<FrameTiming>,
    /// Statistics
    pub stats: AvSyncStats,
}

#[derive(Debug, Default, Clone)]
pub struct AvSyncStats {
    pub frames_presented: u64,
    pub frames_dropped: u64,
    pub frames_repeated: u64,
    pub avg_drift_ms: f64,
    pub max_drift_ms: f64,
}

impl AvSyncState {
    /// Create new A/V sync state for a given frame rate
    pub fn new(fps: f64) -> Self {
        let frame_duration = Time::from_secs(1).div((fps * 1_000_000_000.0) as i64);
        Self {
            frame_duration,
            max_drift_ms: 8.0,     // Allow ±8ms drift before correction
            correction_window: 10, // Look at last 10 frames for correction decisions
            drift_accumulator: 0.0,
            frame_history: VecDeque::with_capacity(50),
            stats: AvSyncStats::default(),
        }
    }

    /// Calculate when the next frame should be presented based on audio master clock
    pub fn next_frame_time(&self, current_time: Time, frame_number: u64) -> Time {
        // Target time = frame_number * frame_duration
        let target_time = self.frame_duration.mul(frame_number as i64);
        // Adjust for any accumulated drift corrections
        let adjusted_time =
            target_time.add(Time::from_millis((self.drift_accumulator * 1000.0) as i64));
        adjusted_time
    }

    /// Decide whether to present, drop, or repeat a frame
    pub fn should_present_frame(&mut self, frame_time: Time, audio_time: Time) -> FrameDecision {
        let drift_ms = (frame_time.as_nanos() as f64 - audio_time.as_nanos() as f64) / 1_000_000.0;

        // Update statistics
        self.stats.max_drift_ms = self.stats.max_drift_ms.max(drift_ms.abs());
        self.stats.avg_drift_ms = (self.stats.avg_drift_ms * 0.9) + (drift_ms.abs() * 0.1);

        // Simple threshold-based decision
        if drift_ms > self.max_drift_ms {
            // Video is behind audio - drop frame to catch up
            FrameDecision::Drop
        } else if drift_ms < -self.max_drift_ms {
            // Video is ahead of audio - repeat frame to slow down
            FrameDecision::Repeat
        } else {
            // Within acceptable range - present normally
            FrameDecision::Present
        }
    }

    /// Record the result of presenting a frame
    pub fn record_frame_presentation(&mut self, timing: FrameTiming) {
        self.frame_history.push_back(timing);
        self.stats.frames_presented += 1;

        // Keep history bounded
        while self.frame_history.len() > 50 {
            self.frame_history.pop_front();
        }

        // Update drift accumulator based on recent history
        if self.frame_history.len() >= self.correction_window {
            let recent_frames: Vec<_> = self
                .frame_history
                .iter()
                .rev()
                .take(self.correction_window)
                .collect();
            let avg_error: f64 = recent_frames
                .iter()
                .filter_map(|t| t.presentation_error())
                .map(|e| e.as_nanos() as f64 / 1_000_000.0) // Convert to ms
                .sum::<f64>()
                / recent_frames.len() as f64;

            // Gradually correct accumulated drift
            self.drift_accumulator = self.drift_accumulator * 0.95 + avg_error * 0.05;
        }
    }

    /// Record a dropped frame
    pub fn record_frame_drop(&mut self) {
        self.stats.frames_dropped += 1;
    }

    /// Record a repeated frame
    pub fn record_frame_repeat(&mut self) {
        self.stats.frames_repeated += 1;
    }

    /// Get current sync statistics
    pub fn stats(&self) -> &AvSyncStats {
        &self.stats
    }
}

/// Decision on how to handle a video frame
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameDecision {
    /// Present the frame normally
    Present,
    /// Drop this frame to catch up
    Drop,
    /// Repeat the previous frame to slow down
    Repeat,
}

/// Video frame scheduler that coordinates with audio master clock
pub struct VideoScheduler<C: Clock> {
    audio_clock: C,
    sync_state: AvSyncState,
    next_frame_number: u64,
    last_presented_time: Option<Time>,
}

impl<C: Clock> VideoScheduler<C> {
    pub fn new(audio_clock: C, fps: f64) -> Self {
        Self {
            audio_clock,
            sync_state: AvSyncState::new(fps),
            next_frame_number: 0,
            last_presented_time: None,
        }
    }

    /// Get the next frame that should be presented
    pub fn next_frame(&mut self) -> Option<ScheduledFrame> {
        let audio_time = self.audio_clock.now();
        let target_time = self
            .sync_state
            .next_frame_time(audio_time, self.next_frame_number);

        // Check if it's time to present this frame
        if audio_time >= target_time {
            let frame = ScheduledFrame {
                frame_number: self.next_frame_number,
                target_time,
                audio_time,
            };

            self.next_frame_number += 1;
            Some(frame)
        } else {
            None
        }
    }

    /// Process the result of presenting a frame
    pub fn frame_presented(&mut self, frame: &ScheduledFrame, actual_presentation_time: Time) {
        let timing = FrameTiming {
            presentation_time: frame.target_time,
            actual_presentation_time: Some(actual_presentation_time),
            duration: self.sync_state.frame_duration,
        };

        self.sync_state.record_frame_presentation(timing);
        self.last_presented_time = Some(actual_presentation_time);
    }

    /// Notify that a frame was dropped
    pub fn frame_dropped(&mut self, frame: &ScheduledFrame) {
        self.sync_state.record_frame_drop();
        self.next_frame_number += 1; // Still advance frame number
    }

    /// Notify that a frame was repeated
    pub fn frame_repeated(&mut self, frame: &ScheduledFrame) {
        self.sync_state.record_frame_repeat();
        // Don't advance frame number - we're repeating
    }

    /// Get current A/V sync statistics
    pub fn stats(&self) -> &AvSyncStats {
        self.sync_state.stats()
    }

    /// Get current drift in milliseconds
    pub fn current_drift_ms(&self) -> f64 {
        if let Some(last_time) = self.last_presented_time {
            let audio_time = self.audio_clock.now();
            (last_time.as_nanos() as f64 - audio_time.as_nanos() as f64) / 1_000_000.0
        } else {
            0.0
        }
    }
}

/// A frame scheduled for presentation
#[derive(Debug, Clone)]
pub struct ScheduledFrame {
    pub frame_number: u64,
    pub target_time: Time,
    pub audio_time: Time,
}

impl ScheduledFrame {
    pub fn drift_ms(&self) -> f64 {
        (self.target_time.as_nanos() as f64 - self.audio_time.as_nanos() as f64) / 1_000_000.0
    }
}

/// High-level A/V sync manager
pub struct AvSyncManager<C: Clock> {
    schedulers: std::collections::HashMap<crate::StreamId, VideoScheduler<C>>,
}

impl<C: Clock + Clone> AvSyncManager<C> {
    pub fn new() -> Self {
        Self {
            schedulers: std::collections::HashMap::new(),
        }
    }

    /// Add a video stream to sync management
    pub fn add_stream(&mut self, stream_id: crate::StreamId, audio_clock: C, fps: f64) {
        self.schedulers
            .insert(stream_id, VideoScheduler::new(audio_clock, fps));
    }

    /// Remove a video stream
    pub fn remove_stream(&mut self, stream_id: &crate::StreamId) {
        self.schedulers.remove(stream_id);
    }

    /// Get next frame for a stream
    pub fn next_frame(&mut self, stream_id: &crate::StreamId) -> Option<ScheduledFrame> {
        self.schedulers.get_mut(stream_id)?.next_frame()
    }

    /// Process frame presentation result
    pub fn frame_presented(
        &mut self,
        stream_id: &crate::StreamId,
        frame: &ScheduledFrame,
        actual_time: Time,
    ) {
        if let Some(scheduler) = self.schedulers.get_mut(stream_id) {
            scheduler.frame_presented(frame, actual_time);
        }
    }

    /// Process frame drop
    pub fn frame_dropped(&mut self, stream_id: &crate::StreamId, frame: &ScheduledFrame) {
        if let Some(scheduler) = self.schedulers.get_mut(stream_id) {
            scheduler.frame_dropped(frame);
        }
    }

    /// Process frame repeat
    pub fn frame_repeated(&mut self, stream_id: &crate::StreamId, frame: &ScheduledFrame) {
        if let Some(scheduler) = self.schedulers.get_mut(stream_id) {
            scheduler.frame_repeated(frame);
        }
    }

    /// Get sync stats for a stream
    pub fn stats(&self, stream_id: &crate::StreamId) -> Option<&AvSyncStats> {
        self.schedulers.get(stream_id).map(|s| s.stats())
    }

    /// Get overall system stats
    pub fn overall_stats(&self) -> AvSyncStats {
        let mut overall = AvSyncStats::default();
        for scheduler in self.schedulers.values() {
            let stats = scheduler.stats();
            overall.frames_presented += stats.frames_presented;
            overall.frames_dropped += stats.frames_dropped;
            overall.frames_repeated += stats.frames_repeated;
            overall.max_drift_ms = overall.max_drift_ms.max(stats.max_drift_ms);
        }
        overall.avg_drift_ms = if !self.schedulers.is_empty() {
            self.schedulers
                .values()
                .map(|s| s.stats().avg_drift_ms)
                .sum::<f64>()
                / self.schedulers.len() as f64
        } else {
            0.0
        };
        overall
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::SystemClock;

    #[test]
    fn test_av_sync_basic() {
        let clock = SystemClock::new();
        let mut sync = AvSyncState::new(30.0); // 30 fps

        let audio_time = Time::from_secs(1);
        let frame_time = Time::from_secs(1);

        let decision = sync.should_present_frame(frame_time, audio_time);
        assert_eq!(decision, FrameDecision::Present);
    }

    #[test]
    fn test_frame_drift_detection() {
        let clock = SystemClock::new();
        let mut sync = AvSyncState::new(30.0);

        // Test case where video is behind (should drop)
        let audio_time = Time::from_secs(1);
        let frame_time = Time::from_millis(1008); // 8ms behind
        let decision = sync.should_present_frame(frame_time, audio_time);
        assert_eq!(decision, FrameDecision::Drop);

        // Test case where video is ahead (should repeat)
        let frame_time = Time::from_millis(992); // 8ms ahead
        let decision = sync.should_present_frame(frame_time, audio_time);
        assert_eq!(decision, FrameDecision::Repeat);
    }

    #[test]
    fn test_video_scheduler() {
        let clock = SystemClock::new();
        let mut scheduler = VideoScheduler::new(clock, 30.0);

        // Initially no frame should be ready
        assert!(scheduler.next_frame().is_none());
    }

    #[test]
    fn test_scheduled_frame_drift() {
        let frame = ScheduledFrame {
            frame_number: 30,
            target_time: Time::from_secs(1), // Frame 30 at 30fps = 1 second
            audio_time: Time::from_millis(1005), // Audio is at 1.005 seconds
        };

        assert_eq!(frame.drift_ms(), -5.0); // Video is 5ms behind audio
    }
}
