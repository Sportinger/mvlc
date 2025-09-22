//! Time management types for MVLC
//!
//! Provides high-precision timing for video playback, frame synchronization,
//! and A/V sync management.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// A high-precision timestamp representing media time in nanoseconds
///
/// This is the fundamental time unit used throughout MVLC for:
/// - Frame timestamps
/// - Playback position
/// - A/V synchronization
/// - Telemetry timing
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Time(pub i64);

impl Time {
    /// Create a Time from nanoseconds
    pub const fn from_nanos(nanos: i64) -> Self {
        Self(nanos)
    }

    /// Create a Time from microseconds
    pub const fn from_micros(micros: i64) -> Self {
        Self(micros * 1_000)
    }

    /// Create a Time from milliseconds
    pub const fn from_millis(millis: i64) -> Self {
        Self(millis * 1_000_000)
    }

    /// Create a Time from seconds
    pub const fn from_secs(secs: i64) -> Self {
        Self(secs * 1_000_000_000)
    }

    /// Create a Time from a Duration
    pub fn from_duration(duration: Duration) -> Self {
        Self(duration.as_nanos() as i64)
    }

    /// Convert to nanoseconds
    pub const fn as_nanos(&self) -> i64 {
        self.0
    }

    /// Convert to microseconds
    pub const fn as_micros(&self) -> i64 {
        self.0 / 1_000
    }

    /// Convert to milliseconds
    pub const fn as_millis(&self) -> i64 {
        self.0 / 1_000_000
    }

    /// Convert to seconds
    pub const fn as_secs(&self) -> i64 {
        self.0 / 1_000_000_000
    }

    /// Convert to Duration
    pub fn as_duration(&self) -> Duration {
        Duration::from_nanos(self.0 as u64)
    }

    /// Zero time
    pub const ZERO: Self = Self(0);

    /// Add two times
    pub const fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }

    /// Subtract two times
    pub const fn sub(self, other: Self) -> Self {
        Self(self.0 - other.0)
    }

    /// Multiply by a scalar
    pub const fn mul(self, scalar: i64) -> Self {
        Self(self.0 * scalar)
    }

    /// Divide by a scalar
    pub const fn div(self, divisor: i64) -> Self {
        Self(self.0 / divisor)
    }
}

impl std::ops::Add for Time {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        self.add(other)
    }
}

impl std::ops::Sub for Time {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        self.sub(other)
    }
}

impl std::ops::AddAssign for Time {
    fn add_assign(&mut self, other: Self) {
        self.0 += other.0;
    }
}

impl std::ops::SubAssign for Time {
    fn sub_assign(&mut self, other: Self) {
        self.0 -= other.0;
    }
}

/// A time range with start and end points
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: Time,
    pub end: Time,
}

impl TimeRange {
    pub fn new(start: Time, end: Time) -> Self {
        assert!(
            start <= end,
            "Start time must be before or equal to end time"
        );
        Self { start, end }
    }

    pub fn duration(&self) -> Time {
        self.end.sub(self.start)
    }

    pub fn contains(&self, time: Time) -> bool {
        time >= self.start && time <= self.end
    }
}

/// Clock interface for time sources
///
/// Provides a way to get the current time, either from system clock
/// or from a master audio clock for A/V synchronization.
pub trait Clock {
    /// Get the current time
    fn now(&self) -> Time;

    /// Get the time since a reference point
    fn elapsed(&self) -> Time {
        self.now()
    }
}

/// System clock implementation using std::time::Instant
pub struct SystemClock {
    start: Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Time {
        Time::from_duration(self.start.elapsed())
    }

    fn elapsed(&self) -> Time {
        self.now()
    }
}

/// Frame timing information for A/V synchronization
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FrameTiming {
    /// When this frame should be presented
    pub presentation_time: Time,
    /// When this frame was actually presented
    pub actual_presentation_time: Option<Time>,
    /// Frame duration
    pub duration: Time,
}

impl FrameTiming {
    pub fn new(presentation_time: Time, duration: Time) -> Self {
        Self {
            presentation_time,
            actual_presentation_time: None,
            duration,
        }
    }

    pub fn mark_presented(&mut self, actual_time: Time) {
        self.actual_presentation_time = Some(actual_time);
    }

    pub fn presentation_error(&self) -> Option<Time> {
        self.actual_presentation_time
            .map(|actual| actual.sub(self.presentation_time))
    }
}
