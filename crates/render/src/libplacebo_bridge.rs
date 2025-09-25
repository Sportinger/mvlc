use anyhow::Result;
#[cfg(feature = "libplacebo")]
use anyhow::{anyhow, Context};
use ash::vk;
use tracing::info;
#[cfg(feature = "libplacebo")]
use tracing::warn;

#[cfg(feature = "libplacebo")]
mod ffi {
    use std::os::raw::{c_int, c_uchar, c_uint};

    #[repr(C)]
    pub struct pl_context {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct pl_renderer {
        _private: [u8; 0],
    }

    extern "C" {
        pub fn pl_context_create() -> *mut pl_context;
        pub fn pl_context_destroy(ctx: *mut pl_context);

        pub fn pl_renderer_create(ctx: *mut pl_context) -> *mut pl_renderer;
        pub fn pl_renderer_destroy(renderer: *mut pl_renderer);

        pub fn pl_renderer_test_quad(renderer: *mut pl_renderer) -> c_int;
        pub fn pl_renderer_test_quad_rgba(
            renderer: *mut pl_renderer,
            width: c_uint,
            height: c_uint,
            dst: *mut c_uchar,
            stride: c_uint,
        ) -> c_int;
    }
}

#[cfg(feature = "libplacebo")]
struct LibplaceboState {
    ctx: *mut ffi::pl_context,
    renderer: *mut ffi::pl_renderer,
}

#[cfg(feature = "libplacebo")]
impl LibplaceboState {
    #[allow(clippy::unused_self)]
    fn new(
        _instance: &ash::Instance,
        _physical_device: vk::PhysicalDevice,
        _device: &ash::Device,
        _queue_family_index: u32,
        _queue: vk::Queue,
    ) -> Result<Self> {
        unsafe {
            let ctx = ffi::pl_context_create();
            if ctx.is_null() {
                return Err(anyhow!("Failed to create libplacebo context"));
            }

            let renderer = ffi::pl_renderer_create(ctx);
            if renderer.is_null() {
                ffi::pl_context_destroy(ctx);
                return Err(anyhow!("Failed to create libplacebo renderer"));
            }

            Ok(Self { ctx, renderer })
        }
    }

    fn render_test_quad_pixels(&mut self, extent: vk::Extent2D) -> Result<Vec<u8>> {
        let width = extent.width;
        let height = extent.height;
        if width == 0 || height == 0 {
            return Err(anyhow!("Swapchain extent is zero"));
        }

        let mut pixels = vec![0u8; (width as usize) * (height as usize) * 4];
        unsafe {
            let ok = ffi::pl_renderer_test_quad_rgba(
                self.renderer,
                width,
                height,
                pixels.as_mut_ptr(),
                width * 4,
            );
            if ok == 0 {
                return Err(anyhow!("libplacebo renderer reported failure"));
            }
        }
        Ok(pixels)
    }
}

#[cfg(feature = "libplacebo")]
impl Drop for LibplaceboState {
    fn drop(&mut self) {
        unsafe {
            ffi::pl_renderer_destroy(self.renderer);
            ffi::pl_context_destroy(self.ctx);
        }
    }
}

#[cfg(feature = "libplacebo")]
pub struct LibplaceboBridge {
    state: Option<LibplaceboState>,
}

#[cfg(not(feature = "libplacebo"))]
pub struct LibplaceboBridge;

#[cfg(feature = "libplacebo")]
impl LibplaceboBridge {
    pub fn new(
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        device: &ash::Device,
        queue_family_index: u32,
        queue: vk::Queue,
    ) -> Result<Self> {
        match LibplaceboState::new(instance, physical_device, device, queue_family_index, queue) {
            Ok(state) => {
                info!("libplacebo bridge initialised (vendor stub)");
                Ok(Self { state: Some(state) })
            }
            Err(err) => {
                warn!("Failed to initialise libplacebo bridge: {err:?}");
                Ok(Self { state: None })
            }
        }
    }

    pub fn is_active(&self) -> bool {
        self.state.is_some()
    }

    pub fn render_test_quad(&mut self, extent: vk::Extent2D) -> Result<Option<Vec<u8>>> {
        match self.state.as_mut() {
            Some(state) => state
                .render_test_quad_pixels(extent)
                .map(Some)
                .context("libplacebo test quad"),
            None => Ok(None),
        }
    }
}

#[cfg(not(feature = "libplacebo"))]
impl LibplaceboBridge {
    #[allow(clippy::unused_self)]
    pub fn new(
        _instance: &ash::Instance,
        _physical_device: vk::PhysicalDevice,
        _device: &ash::Device,
        _queue_family_index: u32,
        _queue: vk::Queue,
    ) -> Result<Self> {
        info!("libplacebo feature disabled; skipping test quad bridge setup");
        Ok(Self)
    }

    pub fn is_active(&self) -> bool {
        false
    }

    #[allow(clippy::unused_self)]
    pub fn render_test_quad(&mut self, _extent: vk::Extent2D) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }
}
