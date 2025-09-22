use anyhow::Result;
use egui::TexturesDelta;
use egui_wgpu::Renderer as EguiRenderer;

pub struct VulkanUiBridge {
    // TODO: hold Vulkan resources for compositing egui output
}

impl VulkanUiBridge {
    pub fn new() -> Result<Self> {
        Ok(Self {})
    }

    pub fn upload_egui_output(
        &mut self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _renderer: &mut EguiRenderer,
        _delta: &TexturesDelta,
    ) -> Result<()> {
        // Phase 2 placeholder: hook egui textures into Vulkan swapchain.
        Ok(())
    }
}
