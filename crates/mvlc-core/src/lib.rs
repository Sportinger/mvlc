//! Core types and traits for MVLC
//!
//! This crate contains the fundamental types used throughout the MVLC application,
//! including time management, layer definitions, project structure, and telemetry.

pub mod time;
pub mod layer;
pub mod project;
pub mod telemetry;

pub use time::*;
pub use layer::*;
pub use project::*;
pub use telemetry::*;
