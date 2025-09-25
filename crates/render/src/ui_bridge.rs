use anyhow::{anyhow, ensure, Context, Result};
use ash::{util::read_spv, vk};
use egui::epaint::{ImageData, ImageDelta, Primitive, TextureId, Vertex as EguiVertex};
use egui::{ClippedPrimitive, FullOutput};
use egui_wgpu::{Renderer as EguiRenderer, ScreenDescriptor};
use std::{collections::HashMap, ffi::CString, io::Cursor, mem::size_of, slice, sync::mpsc};
use wgpu::Maintain;

use crate::vulkan::{FrameContext, VulkanRenderer, MAX_FRAMES_IN_FLIGHT};
use tracing::warn;

pub struct RenderedUiFrame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub enum RenderAction {
    CpuFrame(RenderedUiFrame),
    PresentHandled,
    Skip,
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

const MAX_UI_TEXTURES: u32 = 256;

struct NativeUiBackend {
    device: ash::Device,
    graphics_queue: vk::Queue,
    descriptor_pool: vk::DescriptorPool,
    descriptor_set_layout: vk::DescriptorSetLayout,
    sampler: vk::Sampler,
    pipeline_layout: vk::PipelineLayout,
    pipeline: Option<vk::Pipeline>,
    pipeline_render_pass: Option<vk::RenderPass>,
    per_frame: Vec<PerFrameResources>,
    textures: HashMap<TextureId, UiTexture>,
    upload_command_pool: vk::CommandPool,
    upload_command_buffer: vk::CommandBuffer,
    upload_fence: vk::Fence,
    memory_properties: vk::PhysicalDeviceMemoryProperties,
}

struct RenderSurface {
    texture: wgpu::Texture,
    readback: wgpu::Buffer,
    width: u32,
    height: u32,
    bytes_per_row: usize,
    padded_bytes_per_row: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct GpuVertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: u32,
}

struct UiBuffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    capacity: usize,
}

impl UiBuffer {
    fn new() -> Self {
        Self {
            buffer: vk::Buffer::null(),
            memory: vk::DeviceMemory::null(),
            capacity: 0,
        }
    }

    fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            if self.buffer != vk::Buffer::null() {
                device.destroy_buffer(self.buffer, None);
            }
            if self.memory != vk::DeviceMemory::null() {
                device.free_memory(self.memory, None);
            }
        }
        self.buffer = vk::Buffer::null();
        self.memory = vk::DeviceMemory::null();
        self.capacity = 0;
    }

    fn ensure_capacity(
        &mut self,
        device: &ash::Device,
        mem_props: &vk::PhysicalDeviceMemoryProperties,
        required_bytes: usize,
        usage: vk::BufferUsageFlags,
    ) -> Result<()> {
        if required_bytes == 0 {
            return Ok(());
        }

        if self.capacity >= required_bytes {
            return Ok(());
        }

        self.destroy(device);

        let size = required_bytes as vk::DeviceSize;
        let buffer_info = vk::BufferCreateInfo::builder()
            .size(size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .build();

        let buffer = unsafe { device.create_buffer(&buffer_info, None)? };
        let requirements = unsafe { device.get_buffer_memory_requirements(buffer) };
        let memory_type = find_memory_type(
            mem_props,
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;

        let alloc_info = vk::MemoryAllocateInfo::builder()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type)
            .build();

        let memory = unsafe { device.allocate_memory(&alloc_info, None)? };
        unsafe {
            device.bind_buffer_memory(buffer, memory, 0)?;
        }

        self.buffer = buffer;
        self.memory = memory;
        self.capacity = required_bytes;
        Ok(())
    }

    fn write(&self, device: &ash::Device, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }

        ensure!(
            data.len() <= self.capacity,
            "Buffer overflow: slice {} exceeds capacity {}",
            data.len(),
            self.capacity
        );

        if self.memory == vk::DeviceMemory::null() {
            return Err(anyhow!("Buffer memory not allocated"));
        }

        unsafe {
            let ptr = device
                .map_memory(
                    self.memory,
                    0,
                    data.len() as u64,
                    vk::MemoryMapFlags::empty(),
                )
                .context("Failed to map UI buffer memory")? as *mut u8;
            ptr.copy_from_nonoverlapping(data.as_ptr(), data.len());
            device.unmap_memory(self.memory);
        }

        Ok(())
    }
}

struct PerFrameResources {
    vertex: UiBuffer,
    index: UiBuffer,
}

impl PerFrameResources {
    fn new() -> Self {
        Self {
            vertex: UiBuffer::new(),
            index: UiBuffer::new(),
        }
    }

    fn destroy(&mut self, device: &ash::Device) {
        self.vertex.destroy(device);
        self.index.destroy(device);
    }
}

struct UiTexture {
    image: vk::Image,
    view: vk::ImageView,
    memory: vk::DeviceMemory,
    descriptor_set: vk::DescriptorSet,
    width: u32,
    height: u32,
    layout: vk::ImageLayout,
}

impl UiTexture {
    fn destroy(&mut self, device: &ash::Device, descriptor_pool: vk::DescriptorPool) {
        unsafe {
            if self.view != vk::ImageView::null() {
                device.destroy_image_view(self.view, None);
            }
            if self.image != vk::Image::null() {
                device.destroy_image(self.image, None);
            }
            if self.memory != vk::DeviceMemory::null() {
                device.free_memory(self.memory, None);
            }
            if self.descriptor_set != vk::DescriptorSet::null() {
                let _ = device
                    .free_descriptor_sets(descriptor_pool, slice::from_ref(&self.descriptor_set));
            }
        }

        self.view = vk::ImageView::null();
        self.image = vk::Image::null();
        self.memory = vk::DeviceMemory::null();
        self.descriptor_set = vk::DescriptorSet::null();
        self.layout = vk::ImageLayout::UNDEFINED;
    }
}

struct StagingBuffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    size: vk::DeviceSize,
}

impl StagingBuffer {
    fn new(
        device: &ash::Device,
        mem_props: &vk::PhysicalDeviceMemoryProperties,
        size: vk::DeviceSize,
    ) -> Result<Self> {
        if size == 0 {
            return Err(anyhow!("Cannot allocate zero-sized staging buffer"));
        }

        let buffer_info = vk::BufferCreateInfo::builder()
            .size(size)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .build();

        let buffer = unsafe { device.create_buffer(&buffer_info, None)? };
        let requirements = unsafe { device.get_buffer_memory_requirements(buffer) };
        let memory_type = find_memory_type(
            mem_props,
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;

        let alloc_info = vk::MemoryAllocateInfo::builder()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type)
            .build();

        let memory = unsafe { device.allocate_memory(&alloc_info, None)? };
        unsafe {
            device.bind_buffer_memory(buffer, memory, 0)?;
        }

        Ok(Self {
            buffer,
            memory,
            size,
        })
    }

    fn write(&self, device: &ash::Device, data: &[u8]) -> Result<()> {
        ensure!(
            data.len() as vk::DeviceSize <= self.size,
            "Staging buffer overflow: {} > {}",
            data.len(),
            self.size
        );

        if data.is_empty() {
            return Ok(());
        }

        unsafe {
            let ptr = device
                .map_memory(
                    self.memory,
                    0,
                    data.len() as u64,
                    vk::MemoryMapFlags::empty(),
                )
                .context("Failed to map staging buffer memory")? as *mut u8;
            ptr.copy_from_nonoverlapping(data.as_ptr(), data.len());
            device.unmap_memory(self.memory);
        }

        Ok(())
    }

    fn destroy(self, device: &ash::Device) {
        unsafe {
            device.destroy_buffer(self.buffer, None);
            device.free_memory(self.memory, None);
        }
    }
}

struct DrawCall {
    index_count: u32,
    index_offset: u32,
    vertex_offset: i32,
    scissor: vk::Rect2D,
    texture_id: TextureId,
}

struct DrawBatch {
    vertices: Vec<GpuVertex>,
    indices: Vec<u32>,
    draws: Vec<DrawCall>,
}

impl DrawBatch {
    fn empty() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            draws: Vec::new(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct UiPushConstants {
    scale: [f32; 2],
    translate: [f32; 2],
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

    pub fn promote_to_native(&mut self, vulkan: &mut VulkanRenderer) -> Result<()> {
        self.backend = UiBackend::new_vulkan_native(vulkan)?;
        Ok(())
    }

    pub fn render(
        &mut self,
        vulkan: &mut VulkanRenderer,
        full_output: &FullOutput,
        paint_jobs: &[ClippedPrimitive],
        screen_descriptor: &ScreenDescriptor,
    ) -> Result<()> {
        match self
            .backend
            .render(vulkan, full_output, paint_jobs, screen_descriptor)?
        {
            RenderAction::CpuFrame(frame) => vulkan
                .render_rgba_frame(&frame.pixels, frame.width, frame.height)
                .map(|_| ()),
            RenderAction::PresentHandled => Ok(()),
            RenderAction::Skip => vulkan.render_frame().map(|_| ()),
        }
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
        _vulkan: &mut VulkanRenderer,
        full_output: &FullOutput,
        paint_jobs: &[ClippedPrimitive],
        screen_descriptor: &ScreenDescriptor,
    ) -> Result<RenderAction> {
        let width = screen_descriptor.size_in_pixels[0];
        let height = screen_descriptor.size_in_pixels[1];

        if width == 0 || height == 0 {
            return Ok(RenderAction::Skip);
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

        Ok(RenderAction::CpuFrame(RenderedUiFrame {
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

    fn new_vulkan_native(vulkan: &mut VulkanRenderer) -> Result<Self> {
        Ok(Self::VulkanNative(NativeUiBackend::new(vulkan)?))
    }

    fn render(
        &mut self,
        vulkan: &mut VulkanRenderer,
        full_output: &FullOutput,
        paint_jobs: &[ClippedPrimitive],
        screen_descriptor: &ScreenDescriptor,
    ) -> Result<RenderAction> {
        match self {
            UiBackend::Wgpu(wgpu) => {
                wgpu.render(vulkan, full_output, paint_jobs, screen_descriptor)
            }
            UiBackend::VulkanNative(native) => {
                native.render(vulkan, full_output, paint_jobs, screen_descriptor)
            }
        }
    }
}

impl NativeUiBackend {
    fn new(vulkan: &mut VulkanRenderer) -> Result<Self> {
        let device = vulkan.device().clone();
        let graphics_queue = vulkan.graphics_queue();
        let graphics_queue_family = vulkan.graphics_queue_family_index();
        let memory_properties = unsafe {
            vulkan
                .instance()
                .get_physical_device_memory_properties(vulkan.physical_device())
        };

        let descriptor_pool = Self::create_descriptor_pool(&device)?;
        let descriptor_set_layout = Self::create_descriptor_set_layout(&device)?;
        let sampler = Self::create_sampler(&device)?;
        let pipeline_layout = Self::create_pipeline_layout(&device, descriptor_set_layout)?;
        let upload_command_pool = Self::create_command_pool(&device, graphics_queue_family)?;
        let upload_command_buffer = Self::allocate_command_buffer(&device, upload_command_pool)?;
        let upload_fence = Self::create_fence(&device)?;

        let mut per_frame = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);
        for _ in 0..MAX_FRAMES_IN_FLIGHT {
            per_frame.push(PerFrameResources::new());
        }

        Ok(Self {
            device,
            graphics_queue,
            descriptor_pool,
            descriptor_set_layout,
            sampler,
            pipeline_layout,
            pipeline: None,
            pipeline_render_pass: None,
            per_frame,
            textures: HashMap::new(),
            upload_command_pool,
            upload_command_buffer,
            upload_fence,
            memory_properties,
        })
    }

    fn render(
        &mut self,
        vulkan: &mut VulkanRenderer,
        full_output: &FullOutput,
        paint_jobs: &[ClippedPrimitive],
        screen_descriptor: &ScreenDescriptor,
    ) -> Result<RenderAction> {
        let [width, height] = screen_descriptor.size_in_pixels;
        if width == 0 || height == 0 {
            return Ok(RenderAction::Skip);
        }

        self.ensure_pipeline(vulkan)?;
        self.process_textures(&full_output.textures_delta)?;

        let batch = self.build_draw_batch(paint_jobs, screen_descriptor)?;
        let push_constants = self.push_constants(screen_descriptor);

        vulkan.with_swapchain_frame(
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            move |renderer, ctx| self.record_frame(renderer, ctx, &batch, push_constants),
        )?;

        Ok(RenderAction::PresentHandled)
    }

    fn ensure_pipeline(&mut self, vulkan: &VulkanRenderer) -> Result<()> {
        let render_pass = vulkan.render_pass_handle();
        let needs_rebuild = match self.pipeline_render_pass {
            Some(existing) => existing != render_pass,
            None => true,
        };

        if needs_rebuild {
            self.destroy_pipeline();
            let pipeline = self.create_pipeline(render_pass)?;
            self.pipeline = Some(pipeline);
            self.pipeline_render_pass = Some(render_pass);
        }

        Ok(())
    }

    fn create_pipeline(&self, render_pass: vk::RenderPass) -> Result<vk::Pipeline> {
        let vert_bytes = include_bytes!(concat!(env!("OUT_DIR"), "/ui.vert.spv"));
        let frag_bytes = include_bytes!(concat!(env!("OUT_DIR"), "/ui.frag.spv"));

        let vert_module = Self::create_shader_module(&self.device, vert_bytes)?;
        let frag_module = Self::create_shader_module(&self.device, frag_bytes)?;

        let entry_point = CString::new("main").expect("main entry point");

        let shader_stages = [
            vk::PipelineShaderStageCreateInfo::builder()
                .module(vert_module)
                .stage(vk::ShaderStageFlags::VERTEX)
                .name(entry_point.as_c_str())
                .build(),
            vk::PipelineShaderStageCreateInfo::builder()
                .module(frag_module)
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .name(entry_point.as_c_str())
                .build(),
        ];

        let binding_description = vk::VertexInputBindingDescription::builder()
            .binding(0)
            .stride(size_of::<GpuVertex>() as u32)
            .input_rate(vk::VertexInputRate::VERTEX)
            .build();

        let attribute_descriptions = [
            vk::VertexInputAttributeDescription::builder()
                .binding(0)
                .location(0)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(0)
                .build(),
            vk::VertexInputAttributeDescription::builder()
                .binding(0)
                .location(1)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(8)
                .build(),
            vk::VertexInputAttributeDescription::builder()
                .binding(0)
                .location(2)
                .format(vk::Format::R8G8B8A8_UNORM)
                .offset(16)
                .build(),
        ];

        let vertex_input_state = vk::PipelineVertexInputStateCreateInfo::builder()
            .vertex_binding_descriptions(slice::from_ref(&binding_description))
            .vertex_attribute_descriptions(&attribute_descriptions)
            .build();

        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::builder()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false)
            .build();

        let viewport_state = vk::PipelineViewportStateCreateInfo::builder()
            .viewport_count(1)
            .scissor_count(1)
            .build();

        let rasterization = vk::PipelineRasterizationStateCreateInfo::builder()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0)
            .build();

        let multisample = vk::PipelineMultisampleStateCreateInfo::builder()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1)
            .build();

        let color_blend_attachment = vk::PipelineColorBlendAttachmentState::builder()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::ONE)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(
                vk::ColorComponentFlags::R
                    | vk::ColorComponentFlags::G
                    | vk::ColorComponentFlags::B
                    | vk::ColorComponentFlags::A,
            )
            .build();

        let color_blend = vk::PipelineColorBlendStateCreateInfo::builder()
            .attachments(slice::from_ref(&color_blend_attachment))
            .build();

        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic_state = vk::PipelineDynamicStateCreateInfo::builder()
            .dynamic_states(&dynamic_states)
            .build();

        let pipeline_info = vk::GraphicsPipelineCreateInfo::builder()
            .stages(&shader_stages)
            .vertex_input_state(&vertex_input_state)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterization)
            .multisample_state(&multisample)
            .color_blend_state(&color_blend)
            .dynamic_state(&dynamic_state)
            .layout(self.pipeline_layout)
            .render_pass(render_pass)
            .subpass(0)
            .build();

        let pipeline = unsafe {
            self.device
                .create_graphics_pipelines(
                    vk::PipelineCache::null(),
                    slice::from_ref(&pipeline_info),
                    None,
                )
                .map_err(|(_, err)| anyhow!("Failed to create UI pipeline: {err:?}"))?[0]
        };

        unsafe {
            self.device.destroy_shader_module(vert_module, None);
            self.device.destroy_shader_module(frag_module, None);
        }

        Ok(pipeline)
    }

    fn destroy_pipeline(&mut self) {
        if let Some(pipeline) = self.pipeline.take() {
            unsafe {
                self.device.destroy_pipeline(pipeline, None);
            }
        }
    }

    fn process_textures(&mut self, delta: &egui::TexturesDelta) -> Result<()> {
        for (id, image_delta) in &delta.set {
            self.set_texture(*id, image_delta)?;
        }

        for &id in &delta.free {
            if let Some(mut texture) = self.textures.remove(&id) {
                texture.destroy(&self.device, self.descriptor_pool);
            }
        }

        Ok(())
    }

    fn build_draw_batch(
        &self,
        paint_jobs: &[ClippedPrimitive],
        screen: &ScreenDescriptor,
    ) -> Result<DrawBatch> {
        let mut batch = DrawBatch::empty();
        let pixels_per_point = screen.pixels_per_point;
        let width = screen.size_in_pixels[0];
        let height = screen.size_in_pixels[1];

        for ClippedPrimitive {
            clip_rect,
            primitive,
        } in paint_jobs
        {
            match primitive {
                Primitive::Mesh(mesh) => {
                    if let Some(scissor) =
                        clip_rect_to_vk_rect(clip_rect, pixels_per_point, width, height)
                    {
                        if mesh.indices.is_empty() || mesh.vertices.is_empty() {
                            continue;
                        }

                        let vertex_offset = batch.vertices.len() as i32;
                        let index_offset = batch.indices.len() as u32;

                        batch
                            .vertices
                            .extend(mesh.vertices.iter().map(GpuVertex::from));
                        batch.indices.extend(&mesh.indices);
                        batch.draws.push(DrawCall {
                            index_count: mesh.indices.len() as u32,
                            index_offset,
                            vertex_offset,
                            scissor,
                            texture_id: mesh.texture_id,
                        });
                    }
                }
                Primitive::Callback(_) => {
                    warn!("Skipping unsupported egui callback primitive in Vulkan UI backend");
                }
            }
        }

        Ok(batch)
    }

    fn push_constants(&self, screen: &ScreenDescriptor) -> UiPushConstants {
        let width = screen.size_in_pixels[0].max(1) as f32;
        let height = screen.size_in_pixels[1].max(1) as f32;
        UiPushConstants {
            scale: [2.0 / width, -2.0 / height],
            translate: [-1.0, 1.0],
        }
    }

    fn record_frame(
        &mut self,
        renderer: &mut VulkanRenderer,
        ctx: &FrameContext,
        batch: &DrawBatch,
        push_constants: UiPushConstants,
    ) -> Result<()> {
        self.ensure_buffers_for_frame(ctx.frame_idx, batch)?;
        self.upload_buffers_for_frame(ctx.frame_idx, batch)?;
        self.record_draw_commands(renderer, ctx, batch, push_constants)
    }

    fn ensure_buffers_for_frame(&mut self, frame_idx: usize, batch: &DrawBatch) -> Result<()> {
        if frame_idx >= self.per_frame.len() {
            return Err(anyhow!(
                "Frame index {} exceeds per-frame resources {}",
                frame_idx,
                self.per_frame.len()
            ));
        }

        let vertices_bytes = batch.vertices.len() * size_of::<GpuVertex>();
        let indices_bytes = batch.indices.len() * size_of::<u32>();

        let frame = &mut self.per_frame[frame_idx];
        frame.vertex.ensure_capacity(
            &self.device,
            &self.memory_properties,
            vertices_bytes,
            vk::BufferUsageFlags::VERTEX_BUFFER,
        )?;
        frame.index.ensure_capacity(
            &self.device,
            &self.memory_properties,
            indices_bytes,
            vk::BufferUsageFlags::INDEX_BUFFER,
        )?;

        Ok(())
    }

    fn upload_buffers_for_frame(&self, frame_idx: usize, batch: &DrawBatch) -> Result<()> {
        let frame = &self.per_frame[frame_idx];

        if !batch.vertices.is_empty() {
            let vertex_bytes = unsafe {
                slice::from_raw_parts(
                    batch.vertices.as_ptr() as *const u8,
                    batch.vertices.len() * size_of::<GpuVertex>(),
                )
            };
            frame.vertex.write(&self.device, vertex_bytes)?;
        }

        if !batch.indices.is_empty() {
            let index_bytes = unsafe {
                slice::from_raw_parts(
                    batch.indices.as_ptr() as *const u8,
                    batch.indices.len() * size_of::<u32>(),
                )
            };
            frame.index.write(&self.device, index_bytes)?;
        }

        Ok(())
    }

    fn record_draw_commands(
        &mut self,
        renderer: &mut VulkanRenderer,
        ctx: &FrameContext,
        batch: &DrawBatch,
        push_constants: UiPushConstants,
    ) -> Result<()> {
        let command_buffer = ctx.command_buffer;
        let extent = ctx.extent;

        let begin_info = vk::CommandBufferBeginInfo::builder()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
            .build();

        unsafe {
            self.device
                .begin_command_buffer(command_buffer, &begin_info)?;
        }

        let clear_color = vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.05, 0.05, 0.05, 1.0],
            },
        };

        let render_pass_info = vk::RenderPassBeginInfo::builder()
            .render_pass(renderer.render_pass_handle())
            .framebuffer(ctx.framebuffer)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent,
            })
            .clear_values(slice::from_ref(&clear_color))
            .build();

        unsafe {
            self.device.cmd_begin_render_pass(
                command_buffer,
                &render_pass_info,
                vk::SubpassContents::INLINE,
            );

            if let Some(pipeline) = self.pipeline {
                self.device.cmd_bind_pipeline(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    pipeline,
                );

                let viewport = vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: extent.width as f32,
                    height: extent.height as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                };
                self.device
                    .cmd_set_viewport(command_buffer, 0, slice::from_ref(&viewport));

                let full_scissor = vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent,
                };
                self.device
                    .cmd_set_scissor(command_buffer, 0, slice::from_ref(&full_scissor));

                let push_bytes = slice::from_raw_parts(
                    &push_constants as *const UiPushConstants as *const u8,
                    size_of::<UiPushConstants>(),
                );
                self.device.cmd_push_constants(
                    command_buffer,
                    self.pipeline_layout,
                    vk::ShaderStageFlags::VERTEX,
                    0,
                    push_bytes,
                );

                let frame = &self.per_frame[ctx.frame_idx];
                if frame.vertex.buffer != vk::Buffer::null() {
                    let vertex_buffers = [frame.vertex.buffer];
                    let offsets = [0u64];
                    self.device.cmd_bind_vertex_buffers(
                        command_buffer,
                        0,
                        &vertex_buffers,
                        &offsets,
                    );
                }

                if frame.index.buffer != vk::Buffer::null() {
                    self.device.cmd_bind_index_buffer(
                        command_buffer,
                        frame.index.buffer,
                        0,
                        vk::IndexType::UINT32,
                    );
                }

                for draw in &batch.draws {
                    if let Some(texture) = self.textures.get(&draw.texture_id) {
                        self.device.cmd_set_scissor(
                            command_buffer,
                            0,
                            slice::from_ref(&draw.scissor),
                        );
                        self.device.cmd_bind_descriptor_sets(
                            command_buffer,
                            vk::PipelineBindPoint::GRAPHICS,
                            self.pipeline_layout,
                            0,
                            slice::from_ref(&texture.descriptor_set),
                            &[],
                        );
                        self.device.cmd_draw_indexed(
                            command_buffer,
                            draw.index_count,
                            1,
                            draw.index_offset,
                            draw.vertex_offset,
                            0,
                        );
                    } else {
                        warn!("Missing texture {:?} for egui draw call", draw.texture_id);
                    }
                }
            }

            self.device.cmd_end_render_pass(command_buffer);
            self.device.end_command_buffer(command_buffer)?;
        }

        Ok(())
    }

    fn set_texture(&mut self, id: TextureId, delta: &ImageDelta) -> Result<()> {
        let size = delta.image.size();
        let width = size[0] as u32;
        let height = size[1] as u32;
        let offset = delta
            .pos
            .map(|[x, y]| [x as u32, y as u32])
            .unwrap_or([0, 0]);
        let data = image_to_rgba(&delta.image);

        ensure!(
            data.len() as u32 == width * height * 4,
            "Texture data size mismatch for {:?}: expected {} bytes, got {}",
            id,
            width * height * 4,
            data.len()
        );

        let mut texture = if let Some(existing) = self.textures.remove(&id) {
            existing
        } else {
            self.create_texture_image(width, height)?
        };

        if delta.pos.is_none() && (texture.width != width || texture.height != height) {
            texture.destroy(&self.device, self.descriptor_pool);
            texture = self.create_texture_image(width, height)?;
        }

        if let Err(err) = self.upload_texture_data(&mut texture, offset, width, height, &data) {
            texture.destroy(&self.device, self.descriptor_pool);
            return Err(err);
        }

        self.write_descriptor(&texture);
        self.textures.insert(id, texture);

        Ok(())
    }

    fn create_texture_image(&self, width: u32, height: u32) -> Result<UiTexture> {
        let image_info = vk::ImageCreateInfo::builder()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .build();

        let image = unsafe { self.device.create_image(&image_info, None)? };
        let requirements = unsafe { self.device.get_image_memory_requirements(image) };
        let memory_type = find_memory_type(
            &self.memory_properties,
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;

        let alloc_info = vk::MemoryAllocateInfo::builder()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type)
            .build();

        let memory = unsafe { self.device.allocate_memory(&alloc_info, None)? };
        unsafe {
            self.device.bind_image_memory(image, memory, 0)?;
        }

        let view_info = vk::ImageViewCreateInfo::builder()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .components(vk::ComponentMapping::default())
            .subresource_range(
                vk::ImageSubresourceRange::builder()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1)
                    .build(),
            )
            .build();

        let view = unsafe { self.device.create_image_view(&view_info, None)? };

        let layouts = [self.descriptor_set_layout];
        let alloc_info = vk::DescriptorSetAllocateInfo::builder()
            .descriptor_pool(self.descriptor_pool)
            .set_layouts(&layouts)
            .build();

        let descriptor_set = unsafe { self.device.allocate_descriptor_sets(&alloc_info)?[0] };

        Ok(UiTexture {
            image,
            view,
            memory,
            descriptor_set,
            width,
            height,
            layout: vk::ImageLayout::UNDEFINED,
        })
    }

    fn upload_texture_data(
        &mut self,
        texture: &mut UiTexture,
        offset: [u32; 2],
        width: u32,
        height: u32,
        data: &[u8],
    ) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }

        ensure!(
            offset[0] + width <= texture.width && offset[1] + height <= texture.height,
            "Texture update out of bounds: offset {:?}, size {}x{}, texture {}x{}",
            offset,
            width,
            height,
            texture.width,
            texture.height
        );

        let staging = StagingBuffer::new(
            &self.device,
            &self.memory_properties,
            data.len() as vk::DeviceSize,
        )?;
        staging.write(&self.device, data)?;

        self.begin_upload_commands()?;

        let command_buffer = self.upload_command_buffer;
        let subresource_range = vk::ImageSubresourceRange::builder()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1)
            .build();

        let (src_stage, src_access) = if texture.layout == vk::ImageLayout::UNDEFINED {
            (
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::AccessFlags::empty(),
            )
        } else {
            (
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::AccessFlags::SHADER_READ,
            )
        };

        let barrier_to_transfer = vk::ImageMemoryBarrier::builder()
            .old_layout(texture.layout)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(texture.image)
            .subresource_range(subresource_range)
            .src_access_mask(src_access)
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .build();

        unsafe {
            self.device.cmd_pipeline_barrier(
                command_buffer,
                src_stage,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                slice::from_ref(&barrier_to_transfer),
            );

            let copy_region = vk::BufferImageCopy::builder()
                .buffer_offset(0)
                .buffer_row_length(0)
                .buffer_image_height(0)
                .image_subresource(
                    vk::ImageSubresourceLayers::builder()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .mip_level(0)
                        .base_array_layer(0)
                        .layer_count(1)
                        .build(),
                )
                .image_offset(vk::Offset3D {
                    x: offset[0] as i32,
                    y: offset[1] as i32,
                    z: 0,
                })
                .image_extent(vk::Extent3D {
                    width,
                    height,
                    depth: 1,
                })
                .build();

            self.device.cmd_copy_buffer_to_image(
                command_buffer,
                staging.buffer,
                texture.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                slice::from_ref(&copy_region),
            );

            let barrier_to_shader = vk::ImageMemoryBarrier::builder()
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(texture.image)
                .subresource_range(subresource_range)
                .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ)
                .build();

            self.device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                slice::from_ref(&barrier_to_shader),
            );
        }

        let result = self.end_upload_commands();
        staging.destroy(&self.device);
        result?;
        texture.layout = vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL;
        Ok(())
    }

    fn write_descriptor(&self, texture: &UiTexture) {
        let image_info = vk::DescriptorImageInfo::builder()
            .sampler(self.sampler)
            .image_view(texture.view)
            .image_layout(texture.layout)
            .build();

        let descriptor_write = vk::WriteDescriptorSet::builder()
            .dst_set(texture.descriptor_set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .image_info(slice::from_ref(&image_info))
            .build();

        unsafe {
            self.device
                .update_descriptor_sets(slice::from_ref(&descriptor_write), &[]);
        }
    }

    fn begin_upload_commands(&mut self) -> Result<()> {
        unsafe {
            self.device
                .wait_for_fences(slice::from_ref(&self.upload_fence), true, u64::MAX)?;
            self.device
                .reset_fences(slice::from_ref(&self.upload_fence))?;
            self.device
                .reset_command_pool(self.upload_command_pool, vk::CommandPoolResetFlags::empty())?;
            let begin_info = vk::CommandBufferBeginInfo::builder()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
                .build();
            self.device
                .begin_command_buffer(self.upload_command_buffer, &begin_info)?;
        }
        Ok(())
    }

    fn end_upload_commands(&mut self) -> Result<()> {
        unsafe {
            self.device.end_command_buffer(self.upload_command_buffer)?;
            let command_buffers = [self.upload_command_buffer];
            let submit_info = vk::SubmitInfo::builder()
                .command_buffers(&command_buffers)
                .build();
            self.device.queue_submit(
                self.graphics_queue,
                slice::from_ref(&submit_info),
                self.upload_fence,
            )?;
            self.device
                .wait_for_fences(slice::from_ref(&self.upload_fence), true, u64::MAX)?;
        }
        Ok(())
    }

    fn create_descriptor_pool(device: &ash::Device) -> Result<vk::DescriptorPool> {
        let pool_sizes = [vk::DescriptorPoolSize {
            ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
            descriptor_count: MAX_UI_TEXTURES,
        }];
        let pool_info = vk::DescriptorPoolCreateInfo::builder()
            .pool_sizes(&pool_sizes)
            .max_sets(MAX_UI_TEXTURES)
            .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
            .build();

        let pool = unsafe { device.create_descriptor_pool(&pool_info, None)? };
        Ok(pool)
    }

    fn create_descriptor_set_layout(device: &ash::Device) -> Result<vk::DescriptorSetLayout> {
        let binding = vk::DescriptorSetLayoutBinding::builder()
            .binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT)
            .build();

        let layout_info = vk::DescriptorSetLayoutCreateInfo::builder()
            .bindings(slice::from_ref(&binding))
            .build();

        let layout = unsafe { device.create_descriptor_set_layout(&layout_info, None)? };
        Ok(layout)
    }

    fn create_sampler(device: &ash::Device) -> Result<vk::Sampler> {
        let sampler_info = vk::SamplerCreateInfo::builder()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .border_color(vk::BorderColor::INT_OPAQUE_WHITE)
            .unnormalized_coordinates(false)
            .compare_enable(false)
            .build();

        let sampler = unsafe { device.create_sampler(&sampler_info, None)? };
        Ok(sampler)
    }

    fn create_pipeline_layout(
        device: &ash::Device,
        descriptor_set_layout: vk::DescriptorSetLayout,
    ) -> Result<vk::PipelineLayout> {
        let push_constant_range = vk::PushConstantRange::builder()
            .stage_flags(vk::ShaderStageFlags::VERTEX)
            .offset(0)
            .size(size_of::<UiPushConstants>() as u32)
            .build();

        let layout_info = vk::PipelineLayoutCreateInfo::builder()
            .set_layouts(slice::from_ref(&descriptor_set_layout))
            .push_constant_ranges(slice::from_ref(&push_constant_range))
            .build();

        let layout = unsafe { device.create_pipeline_layout(&layout_info, None)? };
        Ok(layout)
    }

    fn create_command_pool(device: &ash::Device, queue_family: u32) -> Result<vk::CommandPool> {
        let info = vk::CommandPoolCreateInfo::builder()
            .queue_family_index(queue_family)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
            .build();
        Ok(unsafe { device.create_command_pool(&info, None)? })
    }

    fn allocate_command_buffer(
        device: &ash::Device,
        pool: vk::CommandPool,
    ) -> Result<vk::CommandBuffer> {
        let alloc_info = vk::CommandBufferAllocateInfo::builder()
            .command_pool(pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1)
            .build();
        Ok(unsafe { device.allocate_command_buffers(&alloc_info)?[0] })
    }

    fn create_fence(device: &ash::Device) -> Result<vk::Fence> {
        let fence_info = vk::FenceCreateInfo::builder()
            .flags(vk::FenceCreateFlags::SIGNALED)
            .build();
        Ok(unsafe { device.create_fence(&fence_info, None)? })
    }

    fn create_shader_module(device: &ash::Device, bytes: &[u8]) -> Result<vk::ShaderModule> {
        let mut cursor = Cursor::new(bytes);
        let code = read_spv(&mut cursor)?;
        let create_info = vk::ShaderModuleCreateInfo::builder().code(&code).build();
        Ok(unsafe { device.create_shader_module(&create_info, None)? })
    }
}

impl Drop for NativeUiBackend {
    fn drop(&mut self) {
        self.destroy_pipeline();

        for texture in self.textures.values_mut() {
            texture.destroy(&self.device, self.descriptor_pool);
        }

        for frame in &mut self.per_frame {
            frame.destroy(&self.device);
        }

        unsafe {
            if self.pipeline_layout != vk::PipelineLayout::null() {
                self.device
                    .destroy_pipeline_layout(self.pipeline_layout, None);
            }
            self.device.destroy_sampler(self.sampler, None);
            self.device
                .destroy_descriptor_set_layout(self.descriptor_set_layout, None);
            self.device
                .destroy_descriptor_pool(self.descriptor_pool, None);
            self.device.destroy_fence(self.upload_fence, None);
            self.device
                .destroy_command_pool(self.upload_command_pool, None);
        }
    }
}

fn align_to(value: usize, alignment: usize) -> usize {
    ((value + alignment - 1) / alignment) * alignment
}

fn clip_rect_to_vk_rect(
    rect: &egui::Rect,
    pixels_per_point: f32,
    target_width: u32,
    target_height: u32,
) -> Option<vk::Rect2D> {
    let mut min_x = (rect.min.x * pixels_per_point).floor() as i32;
    let mut min_y = (rect.min.y * pixels_per_point).floor() as i32;
    let mut max_x = (rect.max.x * pixels_per_point).ceil() as i32;
    let mut max_y = (rect.max.y * pixels_per_point).ceil() as i32;

    let width = target_width as i32;
    let height = target_height as i32;

    min_x = min_x.clamp(0, width);
    max_x = max_x.clamp(0, width);
    min_y = min_y.clamp(0, height);
    max_y = max_y.clamp(0, height);

    if max_x <= min_x || max_y <= min_y {
        return None;
    }

    Some(vk::Rect2D {
        offset: vk::Offset2D { x: min_x, y: min_y },
        extent: vk::Extent2D {
            width: (max_x - min_x) as u32,
            height: (max_y - min_y) as u32,
        },
    })
}

fn image_to_rgba(image: &ImageData) -> Vec<u8> {
    match image {
        ImageData::Color(color_image) => {
            let mut out = Vec::with_capacity(color_image.pixels.len() * 4);
            for pixel in &color_image.pixels {
                out.extend_from_slice(&pixel.to_array());
            }
            out
        }
        ImageData::Font(font_image) => {
            let mut out = Vec::with_capacity(font_image.pixels.len() * 4);
            for &alpha in &font_image.pixels {
                let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
                out.extend_from_slice(&[a, a, a, a]);
            }
            out
        }
    }
}

fn find_memory_type(
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    type_filter: u32,
    properties: vk::MemoryPropertyFlags,
) -> Result<u32> {
    for index in 0..mem_props.memory_type_count {
        let type_matches = (type_filter & (1 << index)) != 0;
        let has_properties = mem_props.memory_types[index as usize]
            .property_flags
            .contains(properties);

        if type_matches && has_properties {
            return Ok(index);
        }
    }

    Err(anyhow!(
        "Failed to find memory type matching {:?} (filter {:b})",
        properties,
        type_filter
    ))
}

impl From<&EguiVertex> for GpuVertex {
    fn from(vertex: &EguiVertex) -> Self {
        let rgba = vertex.color.to_array();
        Self {
            pos: [vertex.pos.x, vertex.pos.y],
            uv: [vertex.uv.x, vertex.uv.y],
            color: u32::from_le_bytes(rgba),
        }
    }
}
