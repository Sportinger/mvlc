//! Rendering interface for MVLC
//!
//! Provides rendering abstraction for video composition.
//! Currently a placeholder - will be extended with Vulkan/libplacebo.

pub mod color;
pub mod renderer;
pub mod vulkan;
pub mod ui_bridge;

pub use color::*;
pub use renderer::*;
pub use vulkan::*;
pub use ui_bridge::*;
