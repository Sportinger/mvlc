//! Vulkan device management for MVLC
//!
//! Handles Vulkan instance creation, physical device selection,
//! and logical device setup with required extensions.

use ash::vk;
use ash::Entry;
use std::ffi::CStr;
use std::os::raw::c_char;
use tracing::{debug, info, warn};

/// Vulkan device and instance management
pub struct VulkanDevice {
    pub entry: Entry,
    pub instance: ash::Instance,
    pub physical_device: vk::PhysicalDevice,
    pub device: ash::Device,
    pub queue_family_index: u32,
    pub queue: vk::Queue,
}

impl VulkanDevice {
    /// Create a new Vulkan device with the required extensions
    pub fn new(window: &winit::window::Window) -> Result<Self, Box<dyn std::error::Error>> {
        let entry = unsafe { Entry::load()? };

        // Create instance
        let app_info = vk::ApplicationInfo::default()
            .application_name(c"MVLC")
            .application_version(vk::make_api_version(0, 1, 0, 0))
            .engine_name(c"MVLC")
            .engine_version(vk::make_api_version(0, 1, 0, 0))
            .api_version(vk::API_VERSION_1_1);

        let mut extension_names = Self::required_extensions(window)?;
        extension_names.push(ash::ext::debug_utils::NAME.as_ptr());

        let create_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(&extension_names);

        let instance = unsafe { entry.create_instance(&create_info, None)? };

        // Set up debug messenger (optional)
        #[cfg(debug_assertions)]
        {
            let debug_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
                .message_severity(
                    vk::DebugUtilsMessageSeverityFlagsEXT::ERROR |
                    vk::DebugUtilsMessageSeverityFlagsEXT::WARNING |
                    vk::DebugUtilsMessageSeverityFlagsEXT::INFO,
                )
                .message_type(
                    vk::DebugUtilsMessageTypeFlagsEXT::GENERAL |
                    vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION |
                    vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
                )
                .pfn_user_callback(Some(debug_callback));

            let debug_utils = ash::ext::debug_utils::Instance::new(&entry, &instance);
            unsafe {
                debug_utils.create_debug_utils_messenger(&debug_info, None)?;
            }
        }

        // Select physical device
        let physical_device = Self::select_physical_device(&instance)?;

        // Find graphics queue family
        let queue_family_index = Self::find_queue_family(&instance, physical_device)?;

        // Create logical device
        let queue_create_info = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family_index)
            .queue_priorities(&[1.0]);

        let device_extension_names = Self::required_device_extensions();

        let device_create_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(std::slice::from_ref(&queue_create_info))
            .enabled_extension_names(&device_extension_names);

        let device = unsafe { instance.create_device(physical_device, &device_create_info, None)? };

        // Get queue
        let queue = unsafe { device.get_device_queue(queue_family_index, 0) };

        info!("Vulkan device initialized successfully");
        debug!("Physical device: {:?}", physical_device);
        debug!("Queue family index: {}", queue_family_index);

        Ok(Self {
            entry,
            instance,
            physical_device,
            device,
            queue_family_index,
            queue,
        })
    }

    /// Get required instance extensions for the given window
    fn required_extensions(window: &winit::window::Window) -> Result<Vec<*const c_char>, Box<dyn std::error::Error>> {
        let mut extensions = Vec::new();

        // Surface extensions for windowing
        extensions.extend(ash_window::enumerate_required_extensions(window.display_handle()?.as_raw())?);

        // Add debug utils in debug mode
        #[cfg(debug_assertions)]
        extensions.push(ash::ext::debug_utils::NAME.as_ptr());

        Ok(extensions)
    }

    /// Get required device extensions
    fn required_device_extensions() -> Vec<*const c_char> {
        vec![
            ash::khr::swapchain::NAME.as_ptr(),
            // DMA-BUF import extensions will be added here
        ]
    }

    /// Select the best available physical device
    fn select_physical_device(instance: &ash::Instance) -> Result<vk::PhysicalDevice, Box<dyn std::error::Error>> {
        let devices = unsafe { instance.enumerate_physical_devices()? };

        if devices.is_empty() {
            return Err("No Vulkan physical devices found".into());
        }

        // For now, just select the first discrete GPU, or the first device if none found
        for &device in &devices {
            let properties = unsafe { instance.get_physical_device_properties(device) };
            if properties.device_type == vk::PhysicalDeviceType::DISCRETE_GPU {
                info!("Selected discrete GPU: {}", unsafe {
                    CStr::from_ptr(properties.device_name.as_ptr()).to_string_lossy()
                });
                return Ok(device);
            }
        }

        // Fallback to first device
        warn!("No discrete GPU found, using first available device");
        Ok(devices[0])
    }

    /// Find a suitable queue family for graphics operations
    fn find_queue_family(instance: &ash::Instance, physical_device: vk::PhysicalDevice) -> Result<u32, Box<dyn std::error::Error>> {
        let queue_families = unsafe { instance.get_physical_device_queue_family_properties(physical_device) };

        for (i, queue_family) in queue_families.iter().enumerate() {
            if queue_family.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
                return Ok(i as u32);
            }
        }

        Err("No graphics queue family found".into())
    }

    /// Get device memory properties
    pub fn memory_properties(&self) -> vk::PhysicalDeviceMemoryProperties {
        unsafe {
            self.instance.get_physical_device_memory_properties(self.physical_device)
        }
    }

    /// Get device properties
    pub fn properties(&self) -> vk::PhysicalDeviceProperties {
        unsafe {
            self.instance.get_physical_device_properties(self.physical_device)
        }
    }
}

impl Drop for VulkanDevice {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_device(None);
            #[cfg(debug_assertions)]
            {
                // Destroy debug messenger if it was created
                let debug_utils = ash::ext::debug_utils::Instance::new(&self.entry, &self.instance);
                // Note: We would need to store the messenger handle to destroy it properly
            }
            self.instance.destroy_instance(None);
        }
    }
}

/// Debug callback for Vulkan validation layers
#[cfg(debug_assertions)]
unsafe extern "system" fn debug_callback(
    message_severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    message_type: vk::DebugUtilsMessageTypeFlagsEXT,
    p_callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT,
    _user_data: *mut std::os::raw::c_void,
) -> vk::Bool32 {
    let callback_data = *p_callback_data;
    let message_id_number = callback_data.message_id_number;
    let message_id_name = if callback_data.p_message_id_name.is_null() {
        std::borrow::Cow::from("")
    } else {
        std::ffi::CStr::from_ptr(callback_data.p_message_id_name).to_string_lossy()
    };
    let message = if callback_data.p_message.is_null() {
        std::borrow::Cow::from("")
    } else {
        std::ffi::CStr::from_ptr(callback_data.p_message).to_string_lossy()
    };

    match message_severity {
        vk::DebugUtilsMessageSeverityFlagsEXT::ERROR => {
            error!("Vulkan Error [{}]: {}", message_id_name, message);
        }
        vk::DebugUtilsMessageSeverityFlagsEXT::WARNING => {
            warn!("Vulkan Warning [{}]: {}", message_id_name, message);
        }
        vk::DebugUtilsMessageSeverityFlagsEXT::INFO => {
            debug!("Vulkan Info [{}]: {}", message_id_name, message);
        }
        _ => {
            debug!("Vulkan Debug [{}]: {}", message_id_name, message);
        }
    }

    vk::FALSE
}
