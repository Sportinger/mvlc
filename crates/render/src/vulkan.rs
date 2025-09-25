//! Vulkan renderer with swapchain management.
//!
//! This module bootstraps a minimal Vulkan pipeline capable of presenting
//! cleared frames to the window swapchain. It establishes the groundwork for
//! integrating libplacebo and zero-copy interop in later milestones.

use crate::LibplaceboBridge;
use anyhow::{anyhow, Context, Result};
use ash::{vk, Entry};
use ash_window::enumerate_required_extensions;
use raw_window_handle::{
    HasRawDisplayHandle, HasRawWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use std::ffi::CString;
use std::os::raw::c_char;
use std::slice;
use std::time::Duration;
use tracing::{debug, info, warn};
use winit::window::Window;

const MAX_FRAMES_IN_FLIGHT: usize = 2;

pub struct VulkanRenderer {
    entry: Entry,
    instance: ash::Instance,
    surface_loader: ash::extensions::khr::Surface,
    surface: vk::SurfaceKHR,
    physical_device: vk::PhysicalDevice,
    device: ash::Device,
    graphics_queue: vk::Queue,
    present_queue: vk::Queue,
    graphics_queue_family_index: u32,
    present_queue_family_index: u32,
    swapchain_loader: ash::extensions::khr::Swapchain,
    swapchain: vk::SwapchainKHR,
    swapchain_images: Vec<vk::Image>,
    swapchain_image_views: Vec<vk::ImageView>,
    swapchain_format: vk::Format,
    swapchain_extent: vk::Extent2D,
    render_pass: vk::RenderPass,
    framebuffers: Vec<vk::Framebuffer>,
    command_pool: vk::CommandPool,
    command_buffers: Vec<vk::CommandBuffer>,
    image_available_semaphores: Vec<vk::Semaphore>,
    render_finished_semaphores: Vec<vk::Semaphore>,
    in_flight_fences: Vec<vk::Fence>,
    current_frame: usize,
    window_size: vk::Extent2D,
    overlay_buffers: Vec<OverlayBuffer>,
    libplacebo: LibplaceboBridge,
    initialized: bool,
}

struct OverlayBuffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    capacity: vk::DeviceSize,
}

impl OverlayBuffer {
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

    fn write(&self, device: &ash::Device, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }

        let write_size = data.len() as vk::DeviceSize;
        if write_size > self.capacity {
            return Err(anyhow!(
                "Overlay data size {} exceeds buffer capacity {}",
                write_size,
                self.capacity
            ));
        }

        unsafe {
            let mapped_ptr = device
                .map_memory(self.memory, 0, write_size, vk::MemoryMapFlags::empty())
                .with_context(|| "Failed to map overlay buffer memory")?;

            let mapped_slice = slice::from_raw_parts_mut(mapped_ptr as *mut u8, data.len());
            mapped_slice.copy_from_slice(data);
            device.unmap_memory(self.memory);
        }

        Ok(())
    }
}

impl VulkanRenderer {
    pub fn new(window: &Window) -> Result<Self> {
        info!("Bootstrapping Vulkan renderer");

        let entry = unsafe { Entry::load()? };

        let raw_display = window.raw_display_handle();
        let raw_window = window.raw_window_handle();

        let instance = Self::create_instance(&entry, raw_display)?;

        let surface = unsafe {
            ash_window::create_surface(&entry, &instance, raw_display, raw_window, None)?
        };
        let surface_loader = ash::extensions::khr::Surface::new(&entry, &instance);

        let (physical_device, graphics_family, present_family) =
            Self::select_physical_device(&instance, &surface_loader, surface)?;

        let device = Self::create_logical_device(
            &instance,
            physical_device,
            graphics_family,
            present_family,
        )?;

        let graphics_queue = unsafe { device.get_device_queue(graphics_family, 0) };
        let present_queue = unsafe { device.get_device_queue(present_family, 0) };

        let libplacebo = LibplaceboBridge::new(
            &instance,
            physical_device,
            &device,
            graphics_family,
            graphics_queue,
        )?;

        let swapchain_loader = ash::extensions::khr::Swapchain::new(&instance, &device);

        let window_size = vk::Extent2D {
            width: window.inner_size().width.max(1),
            height: window.inner_size().height.max(1),
        };

        let (swapchain, swapchain_images, swapchain_format, swapchain_extent) =
            Self::create_swapchain(
                &device,
                &swapchain_loader,
                physical_device,
                surface,
                graphics_family,
                present_family,
                &surface_loader,
                window_size,
            )?;

        let swapchain_image_views =
            Self::create_image_views(&device, &swapchain_images, swapchain_format)?;
        let render_pass = Self::create_render_pass(&device, swapchain_format)?;
        let framebuffers = Self::create_framebuffers(
            &device,
            &swapchain_image_views,
            render_pass,
            swapchain_extent,
        )?;
        let command_pool = Self::create_command_pool(&device, graphics_family)?;
        let command_buffers =
            Self::allocate_command_buffers(&device, command_pool, framebuffers.len() as u32)?;

        let (image_available_semaphores, render_finished_semaphores, in_flight_fences) =
            Self::create_sync_objects(&device)?;

        let required_overlay_size = (swapchain_extent.width as vk::DeviceSize)
            * (swapchain_extent.height as vk::DeviceSize)
            * 4;
        let overlay_buffers = Self::allocate_overlay_buffers(
            &instance,
            physical_device,
            &device,
            required_overlay_size,
        )?;

        Ok(Self {
            entry,
            instance,
            surface_loader,
            surface,
            physical_device,
            device,
            graphics_queue,
            present_queue,
            graphics_queue_family_index: graphics_family,
            present_queue_family_index: present_family,
            swapchain_loader,
            swapchain,
            swapchain_images,
            swapchain_image_views,
            swapchain_format,
            swapchain_extent,
            render_pass,
            framebuffers,
            command_pool,
            command_buffers,
            image_available_semaphores,
            render_finished_semaphores,
            in_flight_fences,
            current_frame: 0,
            window_size,
            overlay_buffers,
            libplacebo,
            initialized: true,
        })
    }

    pub fn init(&mut self) -> Result<()> {
        // Nothing extra to do; initialization handled in `new`.
        Ok(())
    }

    pub fn is_ready(&self) -> bool {
        self.initialized
    }

    pub fn render_frame(&mut self) -> Result<u64> {
        self.draw_frame()?;
        Ok(0)
    }

    pub fn render_rgba_frame(&mut self, rgba: &[u8], width: u32, height: u32) -> Result<u64> {
        self.draw_rgba_frame(rgba, width, height)?;
        Ok(0)
    }

    pub fn resize(&mut self, new_width: u32, new_height: u32) -> Result<()> {
        let width = new_width.max(1);
        let height = new_height.max(1);
        self.window_size = vk::Extent2D { width, height };
        self.recreate_swapchain()?;
        Ok(())
    }

    fn reallocate_overlay_buffers(&mut self, required_size: vk::DeviceSize) -> Result<()> {
        let mem_props = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        };

        if self.overlay_buffers.len() != MAX_FRAMES_IN_FLIGHT {
            self.overlay_buffers = Self::allocate_overlay_buffers(
                &self.instance,
                self.physical_device,
                &self.device,
                required_size,
            )?;
            return Ok(());
        }

        for buffer in &mut self.overlay_buffers {
            if buffer.capacity < required_size {
                buffer.destroy(&self.device);
                *buffer = Self::create_overlay_buffer(&self.device, &mem_props, required_size)?;
            }
        }

        Ok(())
    }

    fn draw_rgba_frame(&mut self, rgba: &[u8], width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 || rgba.is_empty() {
            return self.draw_frame();
        }

        if self.swapchain_extent.width != width || self.swapchain_extent.height != height {
            self.resize(width, height)?;
        }

        let frame_idx = self.current_frame;
        self.stage_overlay_pixels(frame_idx, rgba, width, height)?;
        let overlay_buffer = &self.overlay_buffers[frame_idx];

        let fence = self.in_flight_fences[frame_idx];
        unsafe {
            self.device.wait_for_fences(
                &[fence],
                true,
                Duration::from_secs(1).as_nanos() as u64,
            )?;
        }

        let (image_index, suboptimal) = unsafe {
            self.swapchain_loader.acquire_next_image(
                self.swapchain,
                Duration::from_millis(500).as_nanos() as u64,
                self.image_available_semaphores[frame_idx],
                vk::Fence::null(),
            )
        }?;

        if suboptimal {
            debug!("Swapchain is suboptimal, triggering recreation");
            self.recreate_swapchain()?;
            return Ok(());
        }

        unsafe {
            self.device.reset_fences(&[fence])?;
        }

        self.record_rgba_command_buffer(
            self.command_buffers[frame_idx],
            image_index as usize,
            overlay_buffer,
            width,
            height,
        )?;

        let wait_semaphores = [self.image_available_semaphores[frame_idx]];
        let wait_stages = [vk::PipelineStageFlags::TRANSFER];
        let signal_semaphores = [self.render_finished_semaphores[frame_idx]];
        let submit_info = vk::SubmitInfo::builder()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&[self.command_buffers[frame_idx]])
            .signal_semaphores(&signal_semaphores)
            .build();

        unsafe {
            self.device
                .queue_submit(self.graphics_queue, &[submit_info], fence)?;
        }

        let present_info = vk::PresentInfoKHR::builder()
            .wait_semaphores(&signal_semaphores)
            .swapchains(slice::from_ref(&self.swapchain))
            .image_indices(slice::from_ref(&image_index))
            .build();

        let present_result = unsafe {
            self.swapchain_loader
                .queue_present(self.present_queue, &present_info)
        };

        match present_result {
            Ok(suboptimal) if suboptimal => {
                debug!("Swapchain present reported suboptimal; recreating");
                self.recreate_swapchain()?;
            }
            Ok(_) => {}
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.recreate_swapchain()?;
            }
            Err(err) => return Err(anyhow!("Failed to present swapchain image: {err:?}")),
        }

        self.current_frame = (self.current_frame + 1) % MAX_FRAMES_IN_FLIGHT;
        Ok(())
    }

    fn stage_overlay_pixels(
        &mut self,
        frame_idx: usize,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<()> {
        let expected_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|v| v.checked_mul(4))
            .ok_or_else(|| anyhow!("RGBA dimensions overflow: {width}x{height}"))?;

        if rgba.len() != expected_len {
            return Err(anyhow!(
                "RGBA data length {} does not match expected {} for {width}x{height}",
                rgba.len(),
                expected_len
            ));
        }

        let required_size = expected_len as vk::DeviceSize;
        self.reallocate_overlay_buffers(required_size)?;

        let overlay_buffer = &mut self.overlay_buffers[frame_idx];
        overlay_buffer.write(&self.device, rgba)?;
        Ok(())
    }

    fn allocate_overlay_buffers(
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        device: &ash::Device,
        required_size: vk::DeviceSize,
    ) -> Result<Vec<OverlayBuffer>> {
        let mem_props = unsafe { instance.get_physical_device_memory_properties(physical_device) };

        let mut buffers = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);
        for _ in 0..MAX_FRAMES_IN_FLIGHT {
            buffers.push(Self::create_overlay_buffer(
                device,
                &mem_props,
                required_size,
            )?);
        }

        Ok(buffers)
    }

    fn create_overlay_buffer(
        device: &ash::Device,
        mem_props: &vk::PhysicalDeviceMemoryProperties,
        required_size: vk::DeviceSize,
    ) -> Result<OverlayBuffer> {
        let buffer_info = vk::BufferCreateInfo::builder()
            .size(required_size)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .build();

        let buffer = unsafe { device.create_buffer(&buffer_info, None)? };
        let requirements = unsafe { device.get_buffer_memory_requirements(buffer) };
        let memory_type_index = Self::find_memory_type(
            mem_props,
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )
        .ok_or_else(|| anyhow!("Failed to find suitable memory type for overlay buffer"))?;

        let allocation_size = requirements.size.max(required_size);

        let allocate_info = vk::MemoryAllocateInfo::builder()
            .allocation_size(allocation_size)
            .memory_type_index(memory_type_index)
            .build();

        let memory = unsafe { device.allocate_memory(&allocate_info, None)? };
        unsafe {
            device.bind_buffer_memory(buffer, memory, 0)?;
        }

        Ok(OverlayBuffer {
            buffer,
            memory,
            capacity: allocation_size,
        })
    }

    fn find_memory_type(
        mem_props: &vk::PhysicalDeviceMemoryProperties,
        type_filter: u32,
        properties: vk::MemoryPropertyFlags,
    ) -> Option<u32> {
        for index in 0..mem_props.memory_type_count {
            let type_matches = (type_filter & (1 << index)) != 0;
            let has_properties = mem_props.memory_types[index as usize]
                .property_flags
                .contains(properties);

            if type_matches && has_properties {
                return Some(index);
            }
        }

        None
    }

    fn draw_frame(&mut self) -> Result<()> {
        let fences = &self.in_flight_fences;
        let idx = self.current_frame;
        let fence_handle = fences[idx];

        unsafe {
            self.device.wait_for_fences(
                &[fence_handle],
                true,
                Duration::from_secs(1).as_nanos() as u64,
            )?;
        }

        let (image_index, suboptimal) = unsafe {
            self.swapchain_loader.acquire_next_image(
                self.swapchain,
                Duration::from_millis(500).as_nanos() as u64,
                self.image_available_semaphores[idx],
                vk::Fence::null(),
            )
        }?;

        if suboptimal {
            debug!("Swapchain is suboptimal, triggering recreation");
            self.recreate_swapchain()?;
            return Ok(());
        }

        let mut use_overlay = false;
        match self.libplacebo.render_test_quad(self.swapchain_extent) {
            Ok(Some(pixels)) => {
                if let Err(err) = self.stage_overlay_pixels(
                    idx,
                    &pixels,
                    self.swapchain_extent.width,
                    self.swapchain_extent.height,
                ) {
                    warn!("Failed to stage libplacebo quad: {err:?}");
                } else {
                    use_overlay = true;
                }
            }
            Ok(None) => {}
            Err(err) => warn!("libplacebo test quad failed: {err:?}"),
        }

        unsafe {
            self.device.reset_fences(&[fence_handle])?;
        }

        if use_overlay {
            let overlay_buffer = &self.overlay_buffers[idx];
            self.record_rgba_command_buffer(
                self.command_buffers[idx],
                image_index as usize,
                overlay_buffer,
                self.swapchain_extent.width,
                self.swapchain_extent.height,
            )?;
        } else {
            self.record_command_buffer(self.command_buffers[idx], image_index as usize)?;
        }

        let wait_semaphores = [self.image_available_semaphores[idx]];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let signal_semaphores = [self.render_finished_semaphores[idx]];

        let submit_info = vk::SubmitInfo::builder()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&[self.command_buffers[idx]])
            .signal_semaphores(&signal_semaphores)
            .build();

        unsafe {
            self.device
                .queue_submit(self.graphics_queue, &[submit_info], fence_handle)?;
        }

        let swapchains = [self.swapchain];
        let image_indices = [image_index];
        let present_info = vk::PresentInfoKHR::builder()
            .wait_semaphores(&signal_semaphores)
            .swapchains(&swapchains)
            .image_indices(&image_indices)
            .build();

        let present_result = unsafe {
            self.swapchain_loader
                .queue_present(self.present_queue, &present_info)
        };

        match present_result {
            Ok(suboptimal) if suboptimal => {
                debug!("Swapchain present reported suboptimal; recreating");
                self.recreate_swapchain()?;
            }
            Ok(_) => {}
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.recreate_swapchain()?;
            }
            Err(err) => return Err(anyhow!("Failed to present swapchain image: {err:?}")),
        }

        self.current_frame = (self.current_frame + 1) % MAX_FRAMES_IN_FLIGHT;
        Ok(())
    }

    fn record_command_buffer(
        &self,
        command_buffer: vk::CommandBuffer,
        image_index: usize,
    ) -> Result<()> {
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
            .render_pass(self.render_pass)
            .framebuffer(self.framebuffers[image_index])
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: self.swapchain_extent,
            })
            .clear_values(std::slice::from_ref(&clear_color))
            .build();

        unsafe {
            self.device.cmd_begin_render_pass(
                command_buffer,
                &render_pass_info,
                vk::SubpassContents::INLINE,
            );

            // No actual draw yet; clear the framebuffer and end render pass.
            self.device.cmd_end_render_pass(command_buffer);
            self.device.end_command_buffer(command_buffer)?;
        }

        Ok(())
    }

    fn record_rgba_command_buffer(
        &self,
        command_buffer: vk::CommandBuffer,
        image_index: usize,
        overlay_buffer: &OverlayBuffer,
        width: u32,
        height: u32,
    ) -> Result<()> {
        let begin_info = vk::CommandBufferBeginInfo::builder()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
            .build();
        unsafe {
            self.device
                .begin_command_buffer(command_buffer, &begin_info)?;
        }

        let image = self.swapchain_images[image_index];
        let subresource_range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };

        let to_transfer = vk::ImageMemoryBarrier::builder()
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(subresource_range)
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .build();

        unsafe {
            self.device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                slice::from_ref(&to_transfer),
            );
        }

        let region = vk::BufferImageCopy {
            buffer_offset: 0,
            buffer_row_length: 0,
            buffer_image_height: 0,
            image_subresource: vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            },
            image_offset: vk::Offset3D { x: 0, y: 0, z: 0 },
            image_extent: vk::Extent3D {
                width,
                height,
                depth: 1,
            },
        };

        unsafe {
            self.device.cmd_copy_buffer_to_image(
                command_buffer,
                overlay_buffer.buffer,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                slice::from_ref(&region),
            );
        }

        let to_present = vk::ImageMemoryBarrier::builder()
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(subresource_range)
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::MEMORY_READ)
            .build();

        unsafe {
            self.device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                slice::from_ref(&to_present),
            );

            self.device.end_command_buffer(command_buffer)?;
        }

        Ok(())
    }

    fn recreate_swapchain(&mut self) -> Result<()> {
        unsafe { self.device.device_wait_idle()? };

        self.cleanup_swapchain();

        let (swapchain, images, format, extent) = Self::create_swapchain(
            &self.device,
            &self.swapchain_loader,
            self.physical_device,
            self.surface,
            self.graphics_queue_family_index,
            self.present_queue_family_index,
            &self.surface_loader,
            self.window_size,
        )?;

        self.swapchain = swapchain;
        self.swapchain_images = images;
        self.swapchain_format = format;
        self.swapchain_extent = extent;
        self.swapchain_image_views =
            Self::create_image_views(&self.device, &self.swapchain_images, self.swapchain_format)?;
        self.render_pass = Self::create_render_pass(&self.device, self.swapchain_format)?;
        self.framebuffers = Self::create_framebuffers(
            &self.device,
            &self.swapchain_image_views,
            self.render_pass,
            self.swapchain_extent,
        )?;
        self.command_buffers = Self::allocate_command_buffers(
            &self.device,
            self.command_pool,
            self.framebuffers.len() as u32,
        )?;

        let required_overlay_size = (self.swapchain_extent.width as vk::DeviceSize)
            * (self.swapchain_extent.height as vk::DeviceSize)
            * 4;
        self.reallocate_overlay_buffers(required_overlay_size)?;

        Ok(())
    }

    fn cleanup_swapchain(&mut self) {
        unsafe {
            for framebuffer in &self.framebuffers {
                self.device.destroy_framebuffer(*framebuffer, None);
            }
            for image_view in &self.swapchain_image_views {
                self.device.destroy_image_view(*image_view, None);
            }
            self.device.destroy_render_pass(self.render_pass, None);
            self.swapchain_loader
                .destroy_swapchain(self.swapchain, None);
        }
    }

    fn create_instance(entry: &Entry, raw_display: RawDisplayHandle) -> Result<ash::Instance> {
        let app_name = CString::new("mvlc").unwrap();
        let engine_name = CString::new("mvlc-engine").unwrap();

        let app_info = vk::ApplicationInfo::builder()
            .application_name(&app_name)
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(&engine_name)
            .engine_version(vk::make_api_version(0, 0, 1, 0))
            .api_version(vk::API_VERSION_1_1)
            .build();

        let required_extensions = enumerate_required_extensions(raw_display)
            .map_err(|e| anyhow!("Unable to enumerate required extensions: {e}"))?;
        let extension_ptrs: Vec<*const c_char> =
            required_extensions.iter().map(|&ext| ext).collect();

        let create_info = vk::InstanceCreateInfo::builder()
            .application_info(&app_info)
            .enabled_extension_names(&extension_ptrs)
            .build();

        let instance = unsafe { entry.create_instance(&create_info, None)? };
        Ok(instance)
    }

    fn select_physical_device(
        instance: &ash::Instance,
        surface_loader: &ash::extensions::khr::Surface,
        surface: vk::SurfaceKHR,
    ) -> Result<(vk::PhysicalDevice, u32, u32)> {
        let devices = unsafe { instance.enumerate_physical_devices()? };
        let device = devices
            .into_iter()
            .find(|&physical_device| {
                Self::find_queue_families(instance, surface_loader, surface, physical_device)
                    .is_some()
            })
            .ok_or_else(|| anyhow!("Failed to find suitable Vulkan physical device"))?;

        let (graphics, present) =
            Self::find_queue_families(instance, surface_loader, surface, device).unwrap();
        Ok((device, graphics, present))
    }

    fn find_queue_families(
        instance: &ash::Instance,
        surface_loader: &ash::extensions::khr::Surface,
        surface: vk::SurfaceKHR,
        device: vk::PhysicalDevice,
    ) -> Option<(u32, u32)> {
        let families = unsafe { instance.get_physical_device_queue_family_properties(device) };

        let mut graphics_family = None;
        let mut present_family = None;

        for (index, family) in families.iter().enumerate() {
            if family.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
                graphics_family = Some(index as u32);
            }

            let present_support = unsafe {
                surface_loader
                    .get_physical_device_surface_support(device, index as u32, surface)
                    .unwrap_or(false)
            };

            if present_support {
                present_family = Some(index as u32);
            }

            if graphics_family.is_some() && present_family.is_some() {
                break;
            }
        }

        match (graphics_family, present_family) {
            (Some(graphics), Some(present)) => Some((graphics, present)),
            _ => None,
        }
    }

    fn create_logical_device(
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        graphics_family: u32,
        present_family: u32,
    ) -> Result<ash::Device> {
        let priorities = [1.0f32];

        let mut queue_infos = vec![vk::DeviceQueueCreateInfo::builder()
            .queue_family_index(graphics_family)
            .queue_priorities(&priorities)
            .build()];

        if graphics_family != present_family {
            queue_infos.push(
                vk::DeviceQueueCreateInfo::builder()
                    .queue_family_index(present_family)
                    .queue_priorities(&priorities)
                    .build(),
            );
        }

        let extensions = [ash::extensions::khr::Swapchain::name().as_ptr()];

        let device_features = vk::PhysicalDeviceFeatures::builder().build();
        let device_info = vk::DeviceCreateInfo::builder()
            .queue_create_infos(&queue_infos)
            .enabled_extension_names(&extensions)
            .enabled_features(&device_features)
            .build();

        let device = unsafe { instance.create_device(physical_device, &device_info, None)? };
        Ok(device)
    }

    fn create_swapchain(
        device: &ash::Device,
        swapchain_loader: &ash::extensions::khr::Swapchain,
        physical_device: vk::PhysicalDevice,
        surface: vk::SurfaceKHR,
        graphics_family: u32,
        present_family: u32,
        surface_loader: &ash::extensions::khr::Surface,
        window_size: vk::Extent2D,
    ) -> Result<(vk::SwapchainKHR, Vec<vk::Image>, vk::Format, vk::Extent2D)> {
        let capabilities = unsafe {
            surface_loader.get_physical_device_surface_capabilities(physical_device, surface)?
        };

        let formats = unsafe {
            surface_loader.get_physical_device_surface_formats(physical_device, surface)?
        };
        let surface_format = formats
            .iter()
            .find(|format| {
                format.format == vk::Format::B8G8R8A8_SRGB
                    && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
            })
            .cloned()
            .unwrap_or_else(|| formats[0]);

        let present_modes = unsafe {
            surface_loader.get_physical_device_surface_present_modes(physical_device, surface)?
        };
        let present_mode = present_modes
            .iter()
            .cloned()
            .find(|&mode| mode == vk::PresentModeKHR::MAILBOX)
            .unwrap_or(vk::PresentModeKHR::FIFO);

        let extent = Self::choose_swap_extent(capabilities, window_size);

        let mut image_count = capabilities.min_image_count + 1;
        if capabilities.max_image_count > 0 && image_count > capabilities.max_image_count {
            image_count = capabilities.max_image_count;
        }

        let image_sharing_mode;
        let queue_family_indices;
        if graphics_family != present_family {
            queue_family_indices = vec![graphics_family, present_family];
            image_sharing_mode = vk::SharingMode::CONCURRENT;
        } else {
            queue_family_indices = vec![];
            image_sharing_mode = vk::SharingMode::EXCLUSIVE;
        }

        let create_info = vk::SwapchainCreateInfoKHR::builder()
            .surface(surface)
            .min_image_count(image_count)
            .image_format(surface_format.format)
            .image_color_space(surface_format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(image_sharing_mode)
            .queue_family_indices(&queue_family_indices)
            .pre_transform(capabilities.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true)
            .build();

        let swapchain = unsafe { swapchain_loader.create_swapchain(&create_info, None)? };
        let images = unsafe { swapchain_loader.get_swapchain_images(swapchain)? };

        Ok((swapchain, images, surface_format.format, extent))
    }

    fn choose_swap_extent(
        capabilities: vk::SurfaceCapabilitiesKHR,
        window_size: vk::Extent2D,
    ) -> vk::Extent2D {
        if capabilities.current_extent.width != u32::MAX {
            capabilities.current_extent
        } else {
            vk::Extent2D {
                width: window_size.width.clamp(
                    capabilities.min_image_extent.width,
                    capabilities.max_image_extent.width,
                ),
                height: window_size.height.clamp(
                    capabilities.min_image_extent.height,
                    capabilities.max_image_extent.height,
                ),
            }
        }
    }

    fn create_image_views(
        device: &ash::Device,
        images: &[vk::Image],
        format: vk::Format,
    ) -> Result<Vec<vk::ImageView>> {
        let mut views = Vec::with_capacity(images.len());
        for &image in images {
            let create_info = vk::ImageViewCreateInfo::builder()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(format)
                .components(vk::ComponentMapping::default())
                .subresource_range(
                    vk::ImageSubresourceRange::builder()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .level_count(1)
                        .layer_count(1)
                        .build(),
                )
                .build();

            let view = unsafe { device.create_image_view(&create_info, None)? };
            views.push(view);
        }

        Ok(views)
    }

    fn create_render_pass(device: &ash::Device, format: vk::Format) -> Result<vk::RenderPass> {
        let color_attachment = vk::AttachmentDescription::builder()
            .format(format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .build();

        let color_attachment_ref = vk::AttachmentReference::builder()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .build();

        let subpass = vk::SubpassDescription::builder()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(std::slice::from_ref(&color_attachment_ref))
            .build();

        let dependency = vk::SubpassDependency::builder()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            )
            .build();

        let render_pass_info = vk::RenderPassCreateInfo::builder()
            .attachments(std::slice::from_ref(&color_attachment))
            .subpasses(std::slice::from_ref(&subpass))
            .dependencies(std::slice::from_ref(&dependency))
            .build();

        let render_pass = unsafe { device.create_render_pass(&render_pass_info, None)? };
        Ok(render_pass)
    }

    fn create_framebuffers(
        device: &ash::Device,
        image_views: &[vk::ImageView],
        render_pass: vk::RenderPass,
        extent: vk::Extent2D,
    ) -> Result<Vec<vk::Framebuffer>> {
        let mut framebuffers = Vec::with_capacity(image_views.len());
        for &view in image_views {
            let attachments = [view];
            let framebuffer_info = vk::FramebufferCreateInfo::builder()
                .render_pass(render_pass)
                .attachments(&attachments)
                .width(extent.width)
                .height(extent.height)
                .layers(1)
                .build();

            let framebuffer = unsafe { device.create_framebuffer(&framebuffer_info, None)? };
            framebuffers.push(framebuffer);
        }

        Ok(framebuffers)
    }

    fn create_command_pool(device: &ash::Device, queue_family: u32) -> Result<vk::CommandPool> {
        let pool_info = vk::CommandPoolCreateInfo::builder()
            .queue_family_index(queue_family)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
            .build();

        let pool = unsafe { device.create_command_pool(&pool_info, None)? };
        Ok(pool)
    }

    fn allocate_command_buffers(
        device: &ash::Device,
        command_pool: vk::CommandPool,
        count: u32,
    ) -> Result<Vec<vk::CommandBuffer>> {
        let alloc_info = vk::CommandBufferAllocateInfo::builder()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(count)
            .build();

        let buffers = unsafe { device.allocate_command_buffers(&alloc_info)? };
        Ok(buffers)
    }

    fn create_sync_objects(
        device: &ash::Device,
    ) -> Result<(Vec<vk::Semaphore>, Vec<vk::Semaphore>, Vec<vk::Fence>)> {
        let semaphore_info = vk::SemaphoreCreateInfo::default();
        let fence_info = vk::FenceCreateInfo::builder()
            .flags(vk::FenceCreateFlags::SIGNALED)
            .build();

        let mut image_available = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);
        let mut render_finished = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);
        let mut fences = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);

        for _ in 0..MAX_FRAMES_IN_FLIGHT {
            unsafe {
                image_available.push(device.create_semaphore(&semaphore_info, None)?);
                render_finished.push(device.create_semaphore(&semaphore_info, None)?);
                fences.push(device.create_fence(&fence_info, None)?);
            }
        }

        Ok((image_available, render_finished, fences))
    }
}

impl Drop for VulkanRenderer {
    fn drop(&mut self) {
        unsafe {
            self.device.device_wait_idle().ok();

            for buffer in &mut self.overlay_buffers {
                buffer.destroy(&self.device);
            }

            for i in 0..MAX_FRAMES_IN_FLIGHT {
                self.device
                    .destroy_semaphore(self.render_finished_semaphores[i], None);
                self.device
                    .destroy_semaphore(self.image_available_semaphores[i], None);
                self.device.destroy_fence(self.in_flight_fences[i], None);
            }

            self.device.destroy_command_pool(self.command_pool, None);
            self.cleanup_swapchain();

            self.surface_loader.destroy_surface(self.surface, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

impl crate::RendererBackend for VulkanRenderer {
    fn init(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        Ok(self.init()?)
    }

    fn render_frame(&mut self) -> Result<u64, Box<dyn std::error::Error>> {
        Ok(self.render_frame()?)
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
        Ok(self.resize(width, height)?)
    }

    fn is_ready(&self) -> bool {
        self.is_ready()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
