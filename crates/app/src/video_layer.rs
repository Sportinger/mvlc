use egui::TextureId;
use egui_wgpu::Renderer as EguiWgpuRenderer;
use mvlc_media::{VideoDecoder, VideoFrame};

/// Runtime state for a video-backed layer
pub(crate) struct VideoLayerState {
    decoder: VideoDecoder,
    gpu: Option<GpuResources>,
}

impl VideoLayerState {
    pub fn new(decoder: VideoDecoder) -> Self {
        Self {
            decoder,
            gpu: None,
        }
    }

    pub fn update_with_latest_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut EguiWgpuRenderer,
    ) -> Option<VideoFrame> {
        let mut latest: Option<VideoFrame> = None;

        while let Some(frame) = self.decoder.try_recv_frame() {
            self.upload_frame(device, queue, renderer, &frame);
            latest = Some(frame);
        }

        latest
    }

    pub fn texture_id(&self) -> Option<TextureId> {
        self.gpu.as_ref().map(|gpu| gpu.texture_id)
    }

    pub fn dimensions(&self) -> Option<(f32, f32)> {
        self.gpu
            .as_ref()
            .map(|gpu| (gpu.width as f32, gpu.height as f32))
    }

    fn upload_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut EguiWgpuRenderer,
        frame: &VideoFrame,
    ) {
        self.ensure_gpu_resources(device, renderer, frame.width, frame.height);

        if let Some(gpu) = self.gpu.as_ref() {
            let bytes_per_row = 4 * frame.width as usize;
            queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &gpu.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &frame.data,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row as u32),
                    rows_per_image: Some(frame.height),
                },
                wgpu::Extent3d {
                    width: frame.width,
                    height: frame.height,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    fn ensure_gpu_resources(
        &mut self,
        device: &wgpu::Device,
        renderer: &mut EguiWgpuRenderer,
        width: u32,
        height: u32,
    ) {
        let needs_recreate = match self.gpu.as_ref() {
            Some(gpu) => gpu.width != width || gpu.height != height,
            None => true,
        };

        if needs_recreate {
            if let Some(mut gpu) = self.gpu.take() {
                renderer.free_texture(&gpu.texture_id);
                gpu.destroy();
            }

            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("mvlc_video_layer"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });

            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let texture_id = renderer.register_native_texture(device, &view, wgpu::FilterMode::Linear);

            self.gpu = Some(GpuResources {
                texture,
                _view: view,
                texture_id,
                width,
                height,
            });
        }
    }
}

struct GpuResources {
    texture: wgpu::Texture,
    _view: wgpu::TextureView,
    texture_id: TextureId,
    width: u32,
    height: u32,
}

impl GpuResources {
    fn destroy(&mut self) {
        // Textures and views are dropped automatically when this struct goes out of scope.
    }
}
