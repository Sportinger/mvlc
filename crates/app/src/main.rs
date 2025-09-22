mod app_state;
mod badges;
mod canvas;
mod media;
mod transport;
mod ui;
mod video_layer;

use crate::app_state::AppState;
use crate::media::handle_dropped_file;
use crate::ui::show_ui;
use anyhow::anyhow;
use egui_wgpu::{Renderer as EguiWgpuRenderer, ScreenDescriptor};
use mvlc_media::{check_dmabuf_support, check_vaapi_support, init as init_gstreamer};
use std::sync::Arc;
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};

struct GraphicsState {
    window: Arc<winit::window::Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    egui_renderer: EguiWgpuRenderer,
}

impl GraphicsState {
    async fn new(window: Arc<winit::window::Window>) -> anyhow::Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::default(),
            dx12_shader_compiler: Default::default(),
            gles_minor_version: wgpu::Gles3MinorVersion::default(),
        });

        let surface = instance.create_surface(window.clone())?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| anyhow!("Failed to find a suitable GPU adapter"))?;

        let device_descriptor = wgpu::DeviceDescriptor {
            label: Some("mvlc-egui-device"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
        };

        let (device, queue) = adapter.request_device(&device_descriptor, None).await?;

        let capabilities = surface.get_capabilities(&adapter);
        let surface_format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| format.is_srgb())
            .unwrap_or_else(|| capabilities.formats[0]);

        let present_mode = capabilities
            .present_modes
            .iter()
            .copied()
            .find(|mode| matches!(mode, wgpu::PresentMode::Fifo | wgpu::PresentMode::AutoVsync))
            .unwrap_or(wgpu::PresentMode::Fifo);

        let alpha_mode = capabilities
            .alpha_modes
            .iter()
            .copied()
            .find(|mode| {
                matches!(
                    mode,
                    wgpu::CompositeAlphaMode::Opaque | wgpu::CompositeAlphaMode::Auto
                )
            })
            .unwrap_or(wgpu::CompositeAlphaMode::Auto);

        let size = window.inner_size();
        let mut config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode,
            desired_maximum_frame_latency: 2,
            view_formats: vec![],
        };

        surface.configure(&device, &config);

        let egui_renderer = EguiWgpuRenderer::new(&device, config.format, None, 1);

        config.width = size.width.max(1);
        config.height = size.height.max(1);

        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            egui_renderer,
        })
    }

    fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }

        self.config.width = new_size.width;
        self.config.height = new_size.height;
        self.surface.configure(&self.device, &self.config);
    }

    fn screen_descriptor(&self) -> ScreenDescriptor {
        ScreenDescriptor {
            size_in_pixels: [self.config.width.max(1), self.config.height.max(1)],
            pixels_per_point: self.window.scale_factor() as f32,
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    if let Err(e) = init_gstreamer() {
        tracing::warn!("Failed to initialize GStreamer: {}", e);
    } else {
        tracing::info!("GStreamer initialized successfully");
        tracing::info!(
            "VA-API hardware decoding available: {}",
            check_vaapi_support()
        );
        tracing::info!(
            "DMA-BUF zero-copy memory available: {}",
            check_dmabuf_support()
        );
    }

    tracing::info!("Starting MVLC application");

    let event_loop = EventLoop::new()?;
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("MVLC - Modern Video Layered Compositor")
            .with_inner_size(winit::dpi::LogicalSize::new(640.0, 480.0))
            .build(&event_loop)?,
    );

    let mut egui_winit = egui_winit::State::new(
        egui::Context::default(),
        egui::ViewportId::default(),
        window.as_ref(),
        None,
        None,
    );

    let mut app_state = AppState::new(window.as_ref());
    let mut graphics_state = pollster::block_on(GraphicsState::new(window.clone()))?;

    event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::Poll);

        match event {
            Event::WindowEvent { event, .. } => {
                let response = egui_winit.on_window_event(window.as_ref(), &event);
                if response.consumed {
                    return;
                }

                match event {
                    WindowEvent::CloseRequested => {
                        tracing::info!("Window close requested, exiting");
                        elwt.exit();
                    }
                    WindowEvent::Resized(size) => {
                        graphics_state.resize(size);
                        window.request_redraw();
                    }
                    WindowEvent::ScaleFactorChanged {
                        scale_factor: _,
                        mut inner_size_writer,
                    } => {
                        let new_size = window.inner_size();
                        graphics_state.resize(new_size);
                        if let Err(e) = inner_size_writer.request_inner_size(new_size) {
                            tracing::debug!("Failed to request inner size update: {:?}", e);
                        }
                        window.request_redraw();
                    }
                    WindowEvent::DroppedFile(path) => {
                        handle_dropped_file(&mut app_state, &path);
                    }
                    WindowEvent::RedrawRequested => {
                        app_state.poll_video_frames(
                            &graphics_state.device,
                            &graphics_state.queue,
                            &mut graphics_state.egui_renderer,
                        );

                        let raw_input = egui_winit.take_egui_input(window.as_ref());
                        let full_output = egui_winit.egui_ctx().run(raw_input, |ctx| {
                            show_ui(ctx, &mut app_state);
                        });

                        egui_winit
                            .handle_platform_output(window.as_ref(), full_output.platform_output);

                        let screen_descriptor = graphics_state.screen_descriptor();
                        let paint_jobs = egui_winit
                            .egui_ctx()
                            .tessellate(full_output.shapes.clone(), full_output.pixels_per_point);

                        let mut encoder = graphics_state.device.create_command_encoder(
                            &wgpu::CommandEncoderDescriptor {
                                label: Some("egui-wgpu encoder"),
                            },
                        );

                        for (id, image_delta) in &full_output.textures_delta.set {
                            graphics_state.egui_renderer.update_texture(
                                &graphics_state.device,
                                &graphics_state.queue,
                                *id,
                                image_delta,
                            );
                        }

                        let user_command_buffers = graphics_state.egui_renderer.update_buffers(
                            &graphics_state.device,
                            &graphics_state.queue,
                            &mut encoder,
                            &paint_jobs,
                            &screen_descriptor,
                        );

                        let frame = match graphics_state.surface.get_current_texture() {
                            Ok(frame) => frame,
                            Err(wgpu::SurfaceError::Lost) => {
                                graphics_state.resize(window.inner_size());
                                window.request_redraw();
                                return;
                            }
                            Err(wgpu::SurfaceError::OutOfMemory) => {
                                tracing::error!("Surface out of memory, exiting application");
                                elwt.exit();
                                return;
                            }
                            Err(err) => {
                                tracing::warn!("Failed to acquire next surface texture: {}", err);
                                window.request_redraw();
                                return;
                            }
                        };

                        let view = frame
                            .texture
                            .create_view(&wgpu::TextureViewDescriptor::default());

                        {
                            let mut render_pass =
                                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                    label: Some("egui-wgpu render pass"),
                                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                        view: &view,
                                        resolve_target: None,
                                        ops: wgpu::Operations {
                                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                                r: 0.0,
                                                g: 0.0,
                                                b: 0.0,
                                                a: 1.0,
                                            }),
                                            store: wgpu::StoreOp::Store,
                                        },
                                    })],
                                    depth_stencil_attachment: None,
                                    timestamp_writes: None,
                                    occlusion_query_set: None,
                                });

                            graphics_state.egui_renderer.render(
                                &mut render_pass,
                                &paint_jobs,
                                &screen_descriptor,
                            );
                        }

                        graphics_state.queue.submit(
                            user_command_buffers
                                .into_iter()
                                .chain(std::iter::once(encoder.finish())),
                        );
                        frame.present();

                        for id in &full_output.textures_delta.free {
                            graphics_state.egui_renderer.free_texture(id);
                        }

                        window.request_redraw();
                    }
                    _ => {}
                }
            }
            Event::AboutToWait => {
                window.request_redraw();
            }
            _ => {}
        }
    })?;

    Ok(())
}
