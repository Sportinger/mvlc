//! Performance monitoring and metrics for MVLC
//!
//! Tracks upload bytes, frame rates, and performance characteristics
//! to validate zero-copy operation and hardware acceleration benefits.

use std::time::{Duration, Instant};
use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use tracing::{debug, info};

/// Performance statistics for a single stream
#[derive(Debug, Clone)]
pub struct StreamPerformance {
    pub stream_id: u64,
    pub frames_processed: u64,
    pub total_bytes_uploaded: u64,
    pub upload_operations: u64,
    pub avg_frame_time: Duration,
    pub max_frame_time: Duration,
    pub is_zero_copy: bool,
    pub last_update: Instant,
}

impl Default for StreamPerformance {
    fn default() -> Self {
        Self {
            stream_id: 0,
            frames_processed: 0,
            total_bytes_uploaded: 0,
            upload_operations: 0,
            avg_frame_time: Duration::from_millis(33), // ~30fps
            max_frame_time: Duration::from_millis(33),
            is_zero_copy: false,
            last_update: Instant::now(),
        }
    }
}

impl StreamPerformance {
    /// Create new performance tracker for a stream
    pub fn new(stream_id: u64) -> Self {
        Self {
            stream_id,
            ..Default::default()
        }
    }

    /// Record frame processing with upload bytes
    pub fn record_frame(&mut self, upload_bytes: u64, frame_time: Duration, is_zero_copy: bool) {
        self.frames_processed += 1;
        self.total_bytes_uploaded += upload_bytes;
        if upload_bytes > 0 {
            self.upload_operations += 1;
        }
        self.is_zero_copy = is_zero_copy;

        // Update frame time statistics
        if frame_time > self.max_frame_time {
            self.max_frame_time = frame_time;
        }

        // Exponential moving average for frame time
        let alpha = 0.1; // Smoothing factor
        let current_avg_ns = self.avg_frame_time.as_nanos() as f64;
        let new_avg_ns = current_avg_ns * (1.0 - alpha) + frame_time.as_nanos() as f64 * alpha;
        self.avg_frame_time = Duration::from_nanos(new_avg_ns as u64);

        self.last_update = Instant::now();
    }

    /// Get average upload bytes per frame
    pub fn avg_upload_bytes_per_frame(&self) -> f64 {
        if self.frames_processed == 0 {
            0.0
        } else {
            self.total_bytes_uploaded as f64 / self.frames_processed as f64
        }
    }

    /// Get upload efficiency (lower is better, 0 = perfect zero-copy)
    pub fn upload_efficiency(&self) -> f64 {
        self.avg_upload_bytes_per_frame()
    }

    /// Check if operating in zero-copy mode (upload bytes ≈ 0)
    pub fn is_zero_copy_mode(&self) -> bool {
        self.upload_efficiency() < 1024.0 // Less than 1KB per frame average
    }

    /// Get performance summary
    pub fn summary(&self) -> PerformanceSummary {
        PerformanceSummary {
            stream_id: self.stream_id,
            frames_processed: self.frames_processed,
            avg_upload_bytes: self.avg_upload_bytes_per_frame(),
            upload_operations: self.upload_operations,
            avg_frame_time_ms: self.avg_frame_time.as_millis() as f64,
            max_frame_time_ms: self.max_frame_time.as_millis() as f64,
            is_zero_copy: self.is_zero_copy_mode(),
            zero_copy_confirmed: self.is_zero_copy,
        }
    }
}

/// Summary of performance metrics
#[derive(Debug, Clone)]
pub struct PerformanceSummary {
    pub stream_id: u64,
    pub frames_processed: u64,
    pub avg_upload_bytes: f64,
    pub upload_operations: u64,
    pub avg_frame_time_ms: f64,
    pub max_frame_time_ms: f64,
    pub is_zero_copy: bool,
    pub zero_copy_confirmed: bool,
}

/// Global performance monitor
#[derive(Debug)]
pub struct PerformanceMonitor {
    streams: HashMap<u64, StreamPerformance>,
    global_start: Instant,
    total_frames_processed: u64,
    total_upload_bytes: u64,
}

impl Default for PerformanceMonitor {
    fn default() -> Self {
        Self {
            streams: HashMap::new(),
            global_start: Instant::now(),
            total_frames_processed: 0,
            total_upload_bytes: 0,
        }
    }
}

impl PerformanceMonitor {
    /// Create new performance monitor
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or create performance tracker for a stream
    pub fn get_stream(&mut self, stream_id: u64) -> &mut StreamPerformance {
        self.streams.entry(stream_id).or_insert_with(|| StreamPerformance::new(stream_id))
    }

    /// Record frame processing for a stream
    pub fn record_frame(&mut self, stream_id: u64, upload_bytes: u64, frame_time: Duration, is_zero_copy: bool) {
        let stream = self.get_stream(stream_id);
        stream.record_frame(upload_bytes, frame_time, is_zero_copy);

        self.total_frames_processed += 1;
        self.total_upload_bytes += upload_bytes;
    }

    /// Get global performance summary
    pub fn global_summary(&self) -> GlobalPerformanceSummary {
        let runtime = self.global_start.elapsed();
        let avg_fps = if runtime.as_secs_f64() > 0.0 {
            self.total_frames_processed as f64 / runtime.as_secs_f64()
        } else {
            0.0
        };

        let avg_upload_bytes = if self.total_frames_processed > 0 {
            self.total_upload_bytes as f64 / self.total_frames_processed as f64
        } else {
            0.0
        };

        let zero_copy_streams = self.streams.values()
            .filter(|s| s.is_zero_copy_mode())
            .count();

        GlobalPerformanceSummary {
            total_runtime_seconds: runtime.as_secs_f64(),
            total_frames_processed: self.total_frames_processed,
            total_upload_bytes: self.total_upload_bytes,
            avg_fps,
            avg_upload_bytes_per_frame: avg_upload_bytes,
            active_streams: self.streams.len(),
            zero_copy_streams,
            is_fully_zero_copy: zero_copy_streams == self.streams.len() && zero_copy_streams > 0,
        }
    }

    /// Get all stream summaries
    pub fn stream_summaries(&self) -> Vec<PerformanceSummary> {
        self.streams.values().map(|s| s.summary()).collect()
    }

    /// Reset all performance statistics
    pub fn reset(&mut self) {
        self.streams.clear();
        self.global_start = Instant::now();
        self.total_frames_processed = 0;
        self.total_upload_bytes = 0;
    }

    /// Check if system is operating in optimal zero-copy mode
    pub fn is_optimal_zero_copy(&self) -> bool {
        let summary = self.global_summary();
        summary.is_fully_zero_copy && summary.avg_upload_bytes_per_frame < 1024.0
    }
}

/// Global performance summary across all streams
#[derive(Debug, Clone)]
pub struct GlobalPerformanceSummary {
    pub total_runtime_seconds: f64,
    pub total_frames_processed: u64,
    pub total_upload_bytes: u64,
    pub avg_fps: f64,
    pub avg_upload_bytes_per_frame: f64,
    pub active_streams: usize,
    pub zero_copy_streams: usize,
    pub is_fully_zero_copy: bool,
}

impl GlobalPerformanceSummary {
    /// Format upload bytes for display
    pub fn format_upload_bytes(&self) -> String {
        if self.avg_upload_bytes_per_frame < 1024.0 {
            format!("{:.1} B/frame", self.avg_upload_bytes_per_frame)
        } else if self.avg_upload_bytes_per_frame < 1024.0 * 1024.0 {
            format!("{:.1} KB/frame", self.avg_upload_bytes_per_frame / 1024.0)
        } else {
            format!("{:.1} MB/frame", self.avg_upload_bytes_per_frame / (1024.0 * 1024.0))
        }
    }

    /// Get performance status message
    pub fn status_message(&self) -> String {
        if self.is_fully_zero_copy {
            format!("🎯 Zero-Copy Optimal: {} (~0 upload)", self.format_upload_bytes())
        } else if self.zero_copy_streams > 0 {
            format!("⚡ Partial Zero-Copy: {} ({} of {} streams)", self.format_upload_bytes(), self.zero_copy_streams, self.active_streams)
        } else {
            format!("📤 Traditional Upload: {}", self.format_upload_bytes())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_stream_performance() {
        let mut perf = StreamPerformance::new(1);

        // Record some frames
        perf.record_frame(0, Duration::from_millis(33), true);  // Zero-copy
        perf.record_frame(0, Duration::from_millis(32), true);  // Zero-copy
        perf.record_frame(1024 * 1024, Duration::from_millis(35), false); // 1MB upload

        assert_eq!(perf.frames_processed, 3);
        assert_eq!(perf.total_bytes_uploaded, 1024 * 1024);
        assert_eq!(perf.upload_operations, 1);
        assert!(perf.is_zero_copy_mode()); // Average < 1KB despite one big upload
    }

    #[test]
    fn test_performance_monitor() {
        let mut monitor = PerformanceMonitor::new();

        // Record frames for different streams
        monitor.record_frame(1, 0, Duration::from_millis(33), true);  // Zero-copy
        monitor.record_frame(1, 0, Duration::from_millis(32), true);  // Zero-copy
        monitor.record_frame(2, 1024 * 1024, Duration::from_millis(35), false); // Upload

        let summary = monitor.global_summary();
        assert_eq!(summary.total_frames_processed, 3);
        assert_eq!(summary.total_upload_bytes, 1024 * 1024);
        assert_eq!(summary.active_streams, 2);
        assert_eq!(summary.zero_copy_streams, 1); // Only stream 1 is zero-copy
    }
}
