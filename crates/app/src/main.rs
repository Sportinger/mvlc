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
use egui::{ClippedPrimitive, Context as EguiContext, FullOutput};
use egui_wgpu::{Renderer as EguiWgpuRenderer, ScreenDescriptor};
use egui_winit::State as EguiWinitState;
use mvlc_media::{check_dmabuf_support, check_vaapi_support, init as init_media};
use mvlc_render::{Renderer, VulkanUiBridge};
use std::{
    env,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    event::{Event, MouseScrollDelta, WindowEvent},
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

struct RuntimeDebug {
    enabled: bool,
    start: Instant,
    last: Instant,
    frames: u64,
    slow_frames: u64,
    worst_frame_ms: f64,
    exit_after: Option<Duration>,
}

impl RuntimeDebug {
    fn new(has_startup_files: bool) -> Self {
        let exit_after = env::var("MVLC_EXIT_AFTER_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_secs);
        let enabled = has_startup_files || env_flag("MVLC_DEBUG") || exit_after.is_some();

        Self {
            enabled,
            start: Instant::now(),
            last: Instant::now(),
            frames: 0,
            slow_frames: 0,
            worst_frame_ms: 0.0,
            exit_after,
        }
    }

    fn after_frame(&mut self, frame_started: Instant, app_state: &AppState) {
        if !self.enabled {
            return;
        }

        let frame_ms = frame_started.elapsed().as_secs_f64() * 1000.0;
        self.frames += 1;
        if frame_ms > 16.7 {
            self.slow_frames += 1;
        }
        self.worst_frame_ms = self.worst_frame_ms.max(frame_ms);

        let elapsed = self.last.elapsed();
        if elapsed < Duration::from_secs(1) {
            return;
        }

        let perf = app_state.performance_monitor.global_summary();
        tracing::info!(
            target: "mvlc_debug",
            "ui_fps={:.1} worst_frame_ms={:.1} slow_frames={} layers={} videos={} playing={} decoded_frames={} decode_fps={:.1} upload={}",
            self.frames as f64 / elapsed.as_secs_f64(),
            self.worst_frame_ms,
            self.slow_frames,
            app_state.project.layers.layers().len(),
            app_state.video_layers.len(),
            app_state.transport.is_playing,
            perf.total_frames_processed,
            perf.avg_fps,
            perf.format_upload_bytes()
        );

        self.last = Instant::now();
        self.frames = 0;
        self.slow_frames = 0;
        self.worst_frame_ms = 0.0;
    }

    fn should_exit(&self) -> bool {
        self.exit_after
            .map(|duration| self.start.elapsed() >= duration)
            .unwrap_or(false)
    }
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
    if vulkan_preview_enabled() {
        return run_vulkan();
    }

    run_wgpu()
}

fn vulkan_preview_enabled() -> bool {
    match std::env::var("MVLC_VULKAN_SWAPCHAIN") {
        Ok(value) => {
            let value = value.to_ascii_lowercase();
            matches!(value.as_str(), "1" | "true" | "yes" | "on")
        }
        Err(_) => false,
    }
}

fn env_flag(name: &str) -> bool {
    env::var(name)
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

fn startup_files() -> Vec<PathBuf> {
    env::args_os().skip(1).map(PathBuf::from).collect()
}

fn run_wgpu() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let startup_files = startup_files();
    let mut runtime_debug = RuntimeDebug::new(!startup_files.is_empty());

    if let Err(e) = init_media() {
        tracing::warn!("Failed to initialize media backend: {}", e);
    } else {
        tracing::info!("Media backend initialized successfully");
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
    let initial_size = window.inner_size();
    app_state.canvas_viewport.size = [initial_size.width as f32, initial_size.height as f32];
    let mut graphics_state = pollster::block_on(GraphicsState::new(window.clone()))?;

    for path in &startup_files {
        if path.exists() {
            handle_dropped_file(&mut app_state, path);
        } else {
            tracing::error!("Startup media file does not exist: {}", path.display());
        }
    }
    app_state.tile_loaded_layers();

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
                    WindowEvent::MouseWheel { delta, .. } => {
                        handle_zoom_input(&mut app_state, delta);
                    }
                    WindowEvent::Resized(size) => {
                        app_state.canvas_viewport.size = [size.width as f32, size.height as f32];
                        graphics_state.resize(size);
                        window.request_redraw();
                    }
                    WindowEvent::ScaleFactorChanged {
                        scale_factor: _,
                        mut inner_size_writer,
                    } => {
                        let new_size = window.inner_size();
                        app_state.canvas_viewport.size =
                            [new_size.width as f32, new_size.height as f32];
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
                        let frame_started = Instant::now();

                        app_state.poll_video_frames(
                            &graphics_state.device,
                            &graphics_state.queue,
                            &mut graphics_state.egui_renderer,
                        );

                        let raw_input = egui_winit.take_egui_input(window.as_ref());
                        let full_output = egui_winit.egui_ctx().run(raw_input, |ctx| {
                            show_ui(ctx, &mut app_state);
                        });

                        let platform_output = full_output.platform_output.clone();
                        egui_winit.handle_platform_output(window.as_ref(), platform_output);

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

                        runtime_debug.after_frame(frame_started, &app_state);
                        if runtime_debug.should_exit() {
                            tracing::info!("MVLC_EXIT_AFTER_SECS reached, exiting");
                            elwt.exit();
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

fn run_vulkan() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    if let Err(e) = init_media() {
        tracing::warn!("Failed to initialize media backend: {}", e);
    } else {
        tracing::info!("Media backend initialized successfully");
        tracing::info!(
            "VA-API hardware decoding available: {}",
            check_vaapi_support()
        );
        tracing::info!(
            "DMA-BUF zero-copy memory available: {}",
            check_dmabuf_support()
        );
    }

    tracing::info!("Starting MVLC Vulkan prototype");

    let event_loop = EventLoop::new()?;
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("MVLC - Vulkan Preview")
            .with_inner_size(winit::dpi::LogicalSize::new(640.0, 480.0))
            .build(&event_loop)?,
    );

    let mut app_state = AppState::new(window.as_ref());
    let mut renderer = Renderer::new_vulkan(window.as_ref())?;
    renderer.init()?;

    let mut egui_ctx = EguiContext::default();
    let mut egui_winit = EguiWinitState::new(
        egui_ctx.clone(),
        egui::ViewportId::default(),
        window.as_ref(),
        None,
        None,
    );
    let mut ui_bridge = VulkanUiBridge::new()?;

    if env::var("MVLC_VULKAN_UI_NATIVE").as_deref() == Ok("1") {
        if let Some(vulkan) = renderer.as_vulkan() {
            match ui_bridge.promote_to_native(vulkan) {
                Ok(()) => tracing::info!("Using Vulkan-native egui backend"),
                Err(err) => {
                    tracing::error!(
                        "Failed to initialize Vulkan-native UI backend, falling back to wgpu: {err:?}"
                    );
                }
            }
        } else {
            tracing::warn!("Vulkan renderer unavailable; cannot enable native UI backend");
        }
    }

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
                        app_state.canvas_viewport.size = [size.width as f32, size.height as f32];
                        if let Some(vulkan) = renderer.as_vulkan() {
                            if let Err(err) = vulkan.resize(size.width, size.height) {
                                tracing::warn!("Failed to resize Vulkan renderer: {err:?}");
                            }
                        }
                        window.request_redraw();
                    }
                    WindowEvent::ScaleFactorChanged { .. } => {
                        window.request_redraw();
                    }
                    WindowEvent::DroppedFile(path) => {
                        handle_dropped_file(&mut app_state, &path);
                    }
                    WindowEvent::MouseWheel { delta, .. } => {
                        handle_zoom_input(&mut app_state, delta);
                    }
                    WindowEvent::RedrawRequested => {
                        let raw_input = egui_winit.take_egui_input(window.as_ref());
                        let full_output: FullOutput = egui_ctx.run(raw_input, |ctx| {
                            show_ui(ctx, &mut app_state);
                        });

                        let platform_output = full_output.platform_output.clone();
                        egui_winit.handle_platform_output(window.as_ref(), platform_output);

                        let size = window.inner_size();
                        let screen_descriptor = ScreenDescriptor {
                            size_in_pixels: [size.width.max(1), size.height.max(1)],
                            pixels_per_point: window.scale_factor() as f32,
                        };

                        let paint_jobs: Vec<ClippedPrimitive> = egui_ctx
                            .tessellate(full_output.shapes.clone(), full_output.pixels_per_point);

                        app_state.poll_video_frames_headless();

                        if let Some(vulkan) = renderer.as_vulkan() {
                            if let Err(err) = ui_bridge.render(
                                vulkan,
                                &full_output,
                                &paint_jobs,
                                &screen_descriptor,
                            ) {
                                tracing::error!("Failed to render UI: {err:?}");
                            }
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

fn handle_zoom_input(app_state: &mut AppState, delta: MouseScrollDelta) {
    let scroll = match delta {
        MouseScrollDelta::LineDelta(_, y) => y,
        MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / 100.0,
    };

    if scroll.abs() < f32::EPSILON {
        return;
    }

    let factor = (1.0 + scroll * 0.1).clamp(0.5, 1.5);
    if (factor - 1.0).abs() > 0.001 {
        app_state.canvas_viewport.zoom_by(factor);
    }
}
