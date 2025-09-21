//! Vulkan swapchain management for MVLC
//!
//! Handles swapchain creation, image acquisition, and presentation
//! for rendering frames to the display.

use ash::vk;
use ash::khr::swapchain;
use std::collections::VecDeque;
use tracing::{debug, info, warn};

/// Swapchain and associated resources
pub struct VulkanSwapchain {
    swapchain_loader: swapchain::Device,
    swapchain: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    image_views: Vec<vk::ImageView>,
    format: vk::Format,
    extent: vk::Extent2D,
    image_available_semaphores: Vec<vk::Semaphore>,
    render_finished_semaphores: Vec<vk::Semaphore>,
    in_flight_fences: Vec<vk::Fence>,
    current_frame: usize,
    max_frames_in_flight: usize,
}

impl VulkanSwapchain {
    /// Create a new swapchain for the given window surface
    pub fn new(
        device: &super::VulkanDevice,
        surface: vk::SurfaceKHR,
        surface_loader: &ash::khr::surface::Instance,
        window_width: u32,
        window_height: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let swapchain_loader = swapchain::Device::new(&device.instance, &device.device);

        // Get surface capabilities and choose settings
        let surface_capabilities = unsafe {
            surface_loader.get_physical_device_surface_capabilities(device.physical_device, surface)?
        };

        let surface_formats = unsafe {
            surface_loader.get_physical_device_surface_formats(device.physical_device, surface)?
        };

        let present_modes = unsafe {
            surface_loader.get_physical_device_surface_present_modes(device.physical_device, surface)?
        };

        // Choose format (prefer SRGB)
        let format = Self::choose_surface_format(&surface_formats);
        let present_mode = Self::choose_present_mode(&present_modes);
        let extent = Self::choose_extent(&surface_capabilities, window_width, window_height);

        let image_count = (surface_capabilities.min_image_count + 1).min(surface_capabilities.max_image_count);

        // Create swapchain
        let create_info = vk::SwapchainCreateInfoKHR::default()
            .surface(surface)
            .min_image_count(image_count)
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(surface_capabilities.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true);

        let swapchain = unsafe { swapchain_loader.create_swapchain(&create_info, None)? };

        // Get swapchain images
        let images = unsafe { swapchain_loader.get_swapchain_images(swapchain)? };

        // Create image views
        let image_views = Self::create_image_views(&device.device, &images, format.format)?;

        // Create synchronization objects
        let max_frames_in_flight = 2;
        let (image_available_semaphores, render_finished_semaphores, in_flight_fences) =
            Self::create_sync_objects(&device.device, max_frames_in_flight)?;

        info!("Vulkan swapchain created with {} images, format {:?}, extent {}x{}",
              images.len(), format.format, extent.width, extent.height);

        Ok(Self {
            swapchain_loader,
            swapchain,
            images,
            image_views,
            format: format.format,
            extent,
            image_available_semaphores,
            render_finished_semaphores,
            in_flight_fences,
            current_frame: 0,
            max_frames_in_flight,
        })
    }

    /// Choose the best surface format
    fn choose_surface_format(available_formats: &[vk::SurfaceFormatKHR]) -> vk::SurfaceFormatKHR {
        // Prefer SRGB formats
        for &format in available_formats {
            if format.format == vk::Format::B8G8R8A8_SRGB && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR {
                return format;
            }
        }

        // Fallback to first available
        available_formats[0]
    }

    /// Choose the best present mode
    fn choose_present_mode(available_present_modes: &[vk::PresentModeKHR]) -> vk::PresentModeKHR {
        // Prefer mailbox for lowest latency
        for &present_mode in available_present_modes {
            if present_mode == vk::PresentModeKHR::MAILBOX {
                return present_mode;
            }
        }

        // Fallback to FIFO (always available)
        vk::PresentModeKHR::FIFO
    }

    /// Choose the swapchain extent
    fn choose_extent(capabilities: &vk::SurfaceCapabilitiesKHR, width: u32, height: u32) -> vk::Extent2D {
        if capabilities.current_extent.width != u32::MAX {
            capabilities.current_extent
        } else {
            vk::Extent2D {
                width: width.clamp(capabilities.min_image_extent.width, capabilities.max_image_extent.width),
                height: height.clamp(capabilities.min_image_extent.height, capabilities.max_image_extent.height),
            }
        }
    }

    /// Create image views for swapchain images
    fn create_image_views(
        device: &ash::Device,
        images: &[vk::Image],
        format: vk::Format,
    ) -> Result<Vec<vk::ImageView>, Box<dyn std::error::Error>> {
        let mut image_views = Vec::with_capacity(images.len());

        for &image in images {
            let create_info = vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(format)
                .components(vk::ComponentMapping::default())
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                });

            let image_view = unsafe { device.create_image_view(&create_info, None)? };
            image_views.push(image_view);
        }

        Ok(image_views)
    }

    /// Create synchronization objects
    fn create_sync_objects(
        device: &ash::Device,
        max_frames_in_flight: usize,
    ) -> Result<(Vec<vk::Semaphore>, Vec<vk::Semaphore>, Vec<vk::Fence>), Box<dyn std::error::Error>> {
        let mut image_available_semaphores = Vec::with_capacity(max_frames_in_flight);
        let mut render_finished_semaphores = Vec::with_capacity(max_frames_in_flight);
        let mut in_flight_fences = Vec::with_capacity(max_frames_in_flight);

        let semaphore_create_info = vk::SemaphoreCreateInfo::default();
        let fence_create_info = vk::FenceCreateInfo::default()
            .flags(vk::FenceCreateFlags::SIGNALED);

        for _ in 0..max_frames_in_flight {
            let image_available = unsafe { device.create_semaphore(&semaphore_create_info, None)? };
            let render_finished = unsafe { device.create_semaphore(&semaphore_create_info, None)? };
            let fence = unsafe { device.create_fence(&fence_create_info, None)? };

            image_available_semaphores.push(image_available);
            render_finished_semaphores.push(render_finished);
            in_flight_fences.push(fence);
        }

        Ok((image_available_semaphores, render_finished_semaphores, in_flight_fences))
    }

    /// Acquire the next swapchain image
    pub fn acquire_next_image(&mut self) -> Result<(u32, bool), Box<dyn std::error::Error>> {
        unsafe {
            self.device.wait_for_fences(
                std::slice::from_ref(&self.in_flight_fences[self.current_frame]),
                true,
                u64::MAX,
            )?;
            self.device.reset_fences(std::slice::from_ref(&self.in_flight_fences[self.current_frame]))?;
        }

        let (image_index, suboptimal) = unsafe {
            self.swapchain_loader.acquire_next_image(
                self.swapchain,
                u64::MAX,
                self.image_available_semaphores[self.current_frame],
                vk::Fence::null(),
            )?
        };

        Ok((image_index, suboptimal))
    }

    /// Present the current frame
    pub fn present(&mut self, image_index: u32) -> Result<bool, Box<dyn std::error::Error>> {
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(std::slice::from_ref(&self.render_finished_semaphores[self.current_frame]))
            .swapchains(std::slice::from_ref(&self.swapchain))
            .image_indices(std::slice::from_ref(&image_index));

        let suboptimal = unsafe {
            self.swapchain_loader.queue_present(self.queue, &present_info)?
        };

        self.current_frame = (self.current_frame + 1) % self.max_frames_in_flight;

        Ok(suboptimal)
    }

    /// Get the current frame's synchronization objects
    pub fn current_frame_sync(&self) -> (&vk::Semaphore, &vk::Semaphore, &vk::Fence) {
        (
            &self.image_available_semaphores[self.current_frame],
            &self.render_finished_semaphores[self.current_frame],
            &self.in_flight_fences[self.current_frame],
        )
    }

    /// Get swapchain format
    pub fn format(&self) -> vk::Format {
        self.format
    }

    /// Get swapchain extent
    pub fn extent(&self) -> vk::Extent2D {
        self.extent
    }

    /// Get swapchain images
    pub fn images(&self) -> &[vk::Image] {
        &self.images
    }

    /// Get swapchain image views
    pub fn image_views(&self) -> &[vk::ImageView] {
        &self.image_views
    }
}

impl Drop for VulkanSwapchain {
    fn drop(&mut self) {
        unsafe {
            for &fence in &self.in_flight_fences {
                self.device.destroy_fence(fence, None);
            }
            for &semaphore in &self.render_finished_semaphores {
                self.device.destroy_semaphore(semaphore, None);
            }
            for &semaphore in &self.image_available_semaphores {
                self.device.destroy_semaphore(semaphore, None);
            }
            for &image_view in &self.image_views {
                self.device.destroy_image_view(image_view, None);
            }
            self.swapchain_loader.destroy_swapchain(self.swapchain, None);
        }
    }
}
