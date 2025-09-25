//! Rendering interface for MVLC
//!
//! Provides rendering abstraction for video composition.
//! Currently a placeholder - will be extended with Vulkan/libplacebo.

pub mod color;
pub mod libplacebo_bridge;
pub mod renderer;
pub mod ui_bridge;
pub mod vulkan;

pub use color::*;
pub use libplacebo_bridge::*;
pub use renderer::*;
pub use ui_bridge::*;
pub use vulkan::*;
