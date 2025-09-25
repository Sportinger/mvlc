use anyhow::{Context, Result};
use egui::{ClippedPrimitive, FullOutput};
use egui_wgpu::{Renderer as EguiRenderer, ScreenDescriptor};
use std::sync::mpsc;
use wgpu::Maintain;

pub struct RenderedUiFrame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub struct VulkanUiBridge {
    backend: UiBackend,
}

enum UiBackend {
    Wgpu(WgpuUiBackend),
    #[allow(dead_code)]
    VulkanNative(NativeUiBackend),
}

struct WgpuUiBackend {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: EguiRenderer,
    surface: Option<RenderSurface>,
}

struct NativeUiBackend {
    _placeholder: (),
}

struct RenderSurface {
    texture: wgpu::Texture,
    readback: wgpu::Buffer,
    width: u32,
    height: u32,
    bytes_per_row: usize,
    padded_bytes_per_row: usize,
}

impl RenderSurface {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Result<Self> {
        let bytes_per_row = width
            .checked_mul(4)
            .map(|v| v as usize)
            .ok_or_else(|| anyhow::anyhow!("RGBA width overflow: {width}"))?;
        let padded_bytes_per_row =
            align_to(bytes_per_row, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize);
        let buffer_size = padded_bytes_per_row
            .checked_mul(height as usize)
            .ok_or_else(|| anyhow::anyhow!("RGBA buffer overflow: {width}x{height}"))?;

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mvlc-vulkan-ui-bridge-surface"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mvlc-vulkan-ui-bridge-readback"),
            size: buffer_size as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Ok(Self {
            texture,
            readback,
            width,
            height,
            bytes_per_row,
            padded_bytes_per_row,
        })
    }

    fn matches(&self, width: u32, height: u32) -> bool {
        self.width == width && self.height == height
    }
}

impl VulkanUiBridge {
    pub fn new() -> Result<Self> {
        let backend = UiBackend::new_wgpu()?;
        Ok(Self { backend })
    }

    pub fn render(
        &mut self,
        full_output: &FullOutput,
        paint_jobs: &[ClippedPrimitive],
        screen_descriptor: &ScreenDescriptor,
    ) -> Result<Option<RenderedUiFrame>> {
        self.backend
            .render(full_output, paint_jobs, screen_descriptor)
    }
}

impl WgpuUiBackend {
    fn new() -> Result<Self> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok_or_else(|| anyhow::anyhow!("Failed to acquire wgpu adapter for Vulkan UI bridge"))?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("mvlc-vulkan-ui-bridge-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
            },
            None,
        ))?;

        let renderer = EguiRenderer::new(&device, wgpu::TextureFormat::Rgba8UnormSrgb, None, 1);

        Ok(Self {
            device,
            queue,
            renderer,
            surface: None,
        })
    }

    fn render(
        &mut self,
        full_output: &FullOutput,
        paint_jobs: &[ClippedPrimitive],
        screen_descriptor: &ScreenDescriptor,
    ) -> Result<Option<RenderedUiFrame>> {
        let width = screen_descriptor.size_in_pixels[0];
        let height = screen_descriptor.size_in_pixels[1];

        if width == 0 || height == 0 {
            return Ok(None);
        }

        self.ensure_surface(width, height)?;

        for (id, delta) in &full_output.textures_delta.set {
            self.renderer
                .update_texture(&self.device, &self.queue, *id, delta);
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("mvlc-vulkan-ui-bridge-encoder"),
            });

        let surface_view = {
            let surface = self.surface.as_ref().expect("surface must be initialized");
            surface
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default())
        };

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mvlc-vulkan-ui-bridge-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            self.renderer
                .render(&mut render_pass, paint_jobs, screen_descriptor);
        }

        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &self
                    .surface
                    .as_ref()
                    .expect("surface must be initialized")
                    .texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &self
                    .surface
                    .as_ref()
                    .expect("surface must be initialized")
                    .readback,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(
                        self.surface
                            .as_ref()
                            .expect("surface must be initialized")
                            .padded_bytes_per_row as u32,
                    ),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(std::iter::once(encoder.finish()));

        let buffer_slice = self
            .surface
            .as_ref()
            .expect("surface must be initialized")
            .readback
            .slice(..);
        let (sender, receiver) = mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device.poll(Maintain::Wait);

        let map_result = receiver
            .recv()
            .context("Failed to receive wgpu map_async result for UI bridge")?;
        map_result.context("Failed to map UI bridge buffer")?;

        let data = buffer_slice.get_mapped_range();
        let (bytes_per_row, padded_bytes_per_row) = {
            let surface = self.surface.as_ref().expect("surface must be initialized");
            (surface.bytes_per_row, surface.padded_bytes_per_row)
        };
        let mut pixels = Vec::with_capacity(bytes_per_row * height as usize);
        for row in 0..height as usize {
            let start = row * padded_bytes_per_row;
            let end = start + bytes_per_row;
            pixels.extend_from_slice(&data[start..end]);
        }
        drop(data);
        self.surface
            .as_ref()
            .expect("surface must be initialized")
            .readback
            .unmap();

        for id in &full_output.textures_delta.free {
            self.renderer.free_texture(id);
        }

        Ok(Some(RenderedUiFrame {
            pixels,
            width,
            height,
        }))
    }

    fn ensure_surface(&mut self, width: u32, height: u32) -> Result<()> {
        let recreate = match self.surface.as_ref() {
            Some(surface) => !surface.matches(width, height),
            None => true,
        };

        if recreate {
            let surface = RenderSurface::new(&self.device, width, height)
                .context("Failed to allocate UI render surface")?;
            self.surface = Some(surface);
        }

        Ok(())
    }
}

impl UiBackend {
    fn new_wgpu() -> Result<Self> {
        Ok(Self::Wgpu(WgpuUiBackend::new()?))
    }

    fn render(
        &mut self,
        full_output: &FullOutput,
        paint_jobs: &[ClippedPrimitive],
        screen_descriptor: &ScreenDescriptor,
    ) -> Result<Option<RenderedUiFrame>> {
        match self {
            UiBackend::Wgpu(wgpu) => wgpu.render(full_output, paint_jobs, screen_descriptor),
            UiBackend::VulkanNative(native) => {
                native.render(full_output, paint_jobs, screen_descriptor)
            }
        }
    }
}

impl NativeUiBackend {
    #[allow(dead_code)]
    fn render(
        &mut self,
        _full_output: &FullOutput,
        _paint_jobs: &[ClippedPrimitive],
        _screen_descriptor: &ScreenDescriptor,
    ) -> Result<Option<RenderedUiFrame>> {
        Ok(None)
    }
}

fn align_to(value: usize, alignment: usize) -> usize {
    ((value + alignment - 1) / alignment) * alignment
}
