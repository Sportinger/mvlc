//! Telemetry system for MVLC
//!
//! Provides structured logging and metrics collection for performance monitoring,
//! debugging, and optimization. Outputs NDJSON events for analysis.

use crate::time::Time;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;

/// Unique identifier for a stream/layer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StreamId(pub u64);

impl StreamId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

/// Telemetry event types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum TelemetryEvent {
    /// Frame processing started
    FrameStart {
        timestamp: Time,
        stream_id: StreamId,
        frame_number: u64,
    },

    /// Frame processing completed
    FrameEnd {
        timestamp: Time,
        stream_id: StreamId,
        frame_number: u64,
        duration_ns: u64,
    },

    /// Stage transition in pipeline
    StageTransition {
        timestamp: Time,
        stream_id: StreamId,
        stage_from: PipelineStage,
        stage_to: PipelineStage,
        duration_ns: Option<u64>,
    },

    /// Performance metrics
    PerformanceMetric {
        timestamp: Time,
        stream_id: StreamId,
        metric_name: String,
        value: f64,
        unit: String,
    },

    /// A/V sync information
    AvSync {
        timestamp: Time,
        stream_id: StreamId,
        audio_time: Time,
        video_time: Time,
        drift_ms: f64,
    },

    /// Queue depth information
    QueueDepth {
        timestamp: Time,
        stream_id: StreamId,
        queue_name: String,
        depth: usize,
    },

    /// Error event
    Error {
        timestamp: Time,
        stream_id: Option<StreamId>,
        error_type: String,
        message: String,
        context: HashMap<String, String>,
    },
}

/// Pipeline processing stages
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineStage {
    Decode,
    ColorConversion,
    Transfer,
    Render,
    Present,
}

/// Telemetry sink for collecting and outputting events
pub trait TelemetrySink: Send + Sync {
    fn record_event(&self, event: TelemetryEvent);
    fn flush(&mut self) {}
}

/// NDJSON file writer implementation
pub struct NdjsonWriter {
    writer: std::sync::Arc<std::sync::Mutex<std::io::BufWriter<std::fs::File>>>,
}

impl NdjsonWriter {
    pub fn new(path: &std::path::Path) -> std::io::Result<Self> {
        let file = std::fs::File::create(path)?;
        let writer = std::io::BufWriter::new(file);
        Ok(Self {
            writer: std::sync::Arc::new(std::sync::Mutex::new(writer)),
        })
    }
}

impl TelemetrySink for NdjsonWriter {
    fn record_event(&self, event: TelemetryEvent) {
        use std::io::Write;

        if let Ok(json) = serde_json::to_string(&event) {
            if let Ok(mut writer) = self.writer.lock() {
                let _ = writeln!(writer, "{}", json);
            }
        }
    }

    fn flush(&mut self) {
        if let Ok(mut writer) = self.writer.lock() {
            let _ = writer.flush();
        }
    }
}

/// Console logger implementation (for debugging)
pub struct ConsoleLogger;

impl TelemetrySink for ConsoleLogger {
    fn record_event(&self, event: TelemetryEvent) {
        println!("TELEMETRY: {:?}", event);
    }
}

/// Composite sink that writes to multiple destinations
pub struct CompositeSink {
    sinks: Vec<Box<dyn TelemetrySink>>,
}

impl CompositeSink {
    pub fn new() -> Self {
        Self { sinks: Vec::new() }
    }

    pub fn add_sink(&mut self, sink: Box<dyn TelemetrySink>) {
        self.sinks.push(sink);
    }
}

impl TelemetrySink for CompositeSink {
    fn record_event(&self, event: TelemetryEvent) {
        for sink in &self.sinks {
            sink.record_event(event.clone());
        }
    }

    fn flush(&mut self) {
        for sink in &mut self.sinks {
            sink.flush();
        }
    }
}

impl Default for CompositeSink {
    fn default() -> Self {
        Self::new()
    }
}

/// Telemetry session with timing utilities
pub struct TelemetrySession {
    sink: Box<dyn TelemetrySink>,
    stream_id: StreamId,
}

impl TelemetrySession {
    pub fn new(sink: Box<dyn TelemetrySink>, stream_id: StreamId) -> Self {
        Self { sink, stream_id }
    }

    pub fn record_event(&self, event: TelemetryEvent) {
        self.sink.record_event(event);
    }

    pub fn start_stage(&self, stage: PipelineStage) -> StageTimer {
        StageTimer::new(self.clone(), stage)
    }

    pub fn record_metric(&self, name: &str, value: f64, unit: &str) {
        self.record_event(TelemetryEvent::PerformanceMetric {
            timestamp: Time::ZERO, // TODO: Use actual system time
            stream_id: self.stream_id,
            metric_name: name.to_string(),
            value,
            unit: unit.to_string(),
        });
    }

    pub fn record_queue_depth(&self, queue_name: &str, depth: usize) {
        self.record_event(TelemetryEvent::QueueDepth {
            timestamp: Time::ZERO, // TODO: Use actual system time
            stream_id: self.stream_id,
            queue_name: queue_name.to_string(),
            depth,
        });
    }

    pub fn record_av_sync(&self, audio_time: Time, video_time: Time) {
        let drift_ms = (video_time.as_nanos() - audio_time.as_nanos()) as f64 / 1_000_000.0;
        self.record_event(TelemetryEvent::AvSync {
            timestamp: Time::ZERO, // TODO: Use actual system time
            stream_id: self.stream_id,
            audio_time,
            video_time,
            drift_ms,
        });
    }
}

impl Clone for TelemetrySession {
    fn clone(&self) -> Self {
        // Note: We can't actually clone the sink, so we create a no-op session
        Self {
            sink: Box::new(ConsoleLogger), // Fallback
            stream_id: self.stream_id,
        }
    }
}

/// Timer for measuring stage durations
pub struct StageTimer {
    session: TelemetrySession,
    stage: PipelineStage,
    start_time: Time,
}

impl StageTimer {
    fn new(session: TelemetrySession, stage: PipelineStage) -> Self {
        let start_time = Time::ZERO; // TODO: Use actual system time

        session.record_event(TelemetryEvent::StageTransition {
            timestamp: start_time,
            stream_id: session.stream_id,
            stage_from: PipelineStage::Decode, // TODO: Track previous stage
            stage_to: stage,
            duration_ns: None,
        });

        Self {
            session,
            stage,
            start_time,
        }
    }
}

impl Drop for StageTimer {
    fn drop(&mut self) {
        let end_time = Time::ZERO; // TODO: Use actual system time
        let duration_ns = (end_time.as_nanos() - self.start_time.as_nanos()) as u64;

        self.session.record_event(TelemetryEvent::StageTransition {
            timestamp: end_time,
            stream_id: self.session.stream_id,
            stage_from: self.stage,
            stage_to: PipelineStage::Present, // TODO: Track next stage
            duration_ns: Some(duration_ns),
        });
    }
}
