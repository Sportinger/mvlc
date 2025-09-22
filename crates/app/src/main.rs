use egui_wgpu::{Renderer as EguiWgpuRenderer, ScreenDescriptor};
use std::sync::Arc;
use mvlc_core::{Project, Time, LayerId, Layer, AvSyncManager, StreamId, PerformanceMonitor};
use mvlc_media::{AudioOutput, init as init_gstreamer, check_vaapi_support, check_dmabuf_support};
use mvlc_render::{Renderer, ColorPipeline};
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};

/// Runtime badge states for visual path transparency
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BadgeState {
    Optimal,
    Partial,
    Fallback,
}

impl BadgeState {
    pub fn color(&self) -> egui::Color32 {
        match self {
            BadgeState::Optimal => egui::Color32::from_rgb(34, 197, 94),   // Green
            BadgeState::Partial => egui::Color32::from_rgb(251, 191, 36),  // Yellow
            BadgeState::Fallback => egui::Color32::from_rgb(239, 68, 68),  // Red
        }
    }

    pub fn text(&self) -> &'static str {
        match self {
            BadgeState::Optimal => "✓",
            BadgeState::Partial => "⚠",
            BadgeState::Fallback => "✗",
        }
    }
}

/// Runtime badge information
#[derive(Debug, Clone)]
pub struct RuntimeBadge {
    pub name: String,
    pub value: String,
    pub state: BadgeState,
}

impl RuntimeBadge {
    pub fn new(name: String, value: String, state: BadgeState) -> Self {
        Self { name, value, state }
    }
}

/// Canvas viewport and interaction state
#[derive(Debug, Clone)]
pub struct CanvasViewport {
    pub offset: [f32; 2],      // Pan offset
    pub zoom: f32,             // Zoom level (1.0 = 100%)
    pub size: [f32; 2],        // Canvas size in pixels
}

impl Default for CanvasViewport {
    fn default() -> Self {
        Self {
            offset: [0.0, 0.0],
            zoom: 1.0,
            size: [1920.0, 1080.0], // Default canvas size
        }
    }
}

/// Transform handle types for layer manipulation
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransformHandle {
    ScaleTopLeft,
    ScaleTopRight,
    ScaleBottomLeft,
    ScaleBottomRight,
    Rotate,
}

/// Canvas interaction state
#[derive(Debug, Clone)]
pub struct CanvasInteraction {
    pub selected_layer: Option<LayerId>,
    pub is_dragging: bool,
    pub drag_start: [f32; 2],
    pub gizmo_handle: Option<mvlc_core::GizmoHandle>,
}

impl CanvasViewport {
    /// Convert screen coordinates to canvas coordinates
    pub fn screen_to_canvas(&self, screen_pos: [f32; 2], canvas_rect: egui::Rect) -> [f32; 2] {
        let canvas_center = [
            canvas_rect.center().x,
            canvas_rect.center().y,
        ];

        [
            (screen_pos[0] - canvas_center[0]) / self.zoom + self.offset[0],
            (screen_pos[1] - canvas_center[1]) / self.zoom + self.offset[1],
        ]
    }

    /// Convert canvas coordinates to screen coordinates
    pub fn canvas_to_screen(&self, canvas_pos: [f32; 2], canvas_rect: egui::Rect) -> [f32; 2] {
        let canvas_center = [
            canvas_rect.center().x,
            canvas_rect.center().y,
        ];

        [
            (canvas_pos[0] - self.offset[0]) * self.zoom + canvas_center[0],
            (canvas_pos[1] - self.offset[1]) * self.zoom + canvas_center[1],
        ]
    }

    /// Check if a canvas position is visible in the current viewport
    pub fn is_visible(&self, canvas_pos: [f32; 2], canvas_rect: egui::Rect) -> bool {
        let screen_pos = self.canvas_to_screen(canvas_pos, canvas_rect);
        canvas_rect.contains(egui::pos2(screen_pos[0], screen_pos[1]))
    }
}

impl CanvasInteraction {
    /// Check if a point hits a layer
    pub fn hit_test(&self, canvas_pos: [f32; 2], layer: &Layer, _viewport: &CanvasViewport) -> bool {
        // Use the layer's built-in hit testing with placeholder video dimensions
        let video_width = 1920.0;  // Assume 1080p video
        let video_height = 1080.0;
        layer.contains_point(canvas_pos, video_width, video_height)
    }

    /// Find the gizmo handle at a given canvas position for a layer
    pub fn hit_test_handle(&self, canvas_pos: [f32; 2], layer: &Layer, handle_size: f32) -> Option<mvlc_core::GizmoHandle> {
        if !layer.visible || !layer.selected {
            return None;
        }

        // Use the layer's built-in gizmo detection
        let video_width = 1920.0;  // Assume 1080p video
        let video_height = 1080.0;
        let handle = layer.gizmo_handle_at(canvas_pos, video_width, video_height, handle_size);

        if handle != mvlc_core::GizmoHandle::None {
            Some(handle)
        } else {
            None
        }
    }

    /// Start dragging a layer or gizmo handle
    pub fn start_drag(&mut self, canvas_pos: [f32; 2], layer: &Layer, handle_size: f32) {
        self.is_dragging = true;
        self.drag_start = canvas_pos;

        // Check for gizmo handles first
        if let Some(handle) = self.hit_test_handle(canvas_pos, layer, handle_size) {
            self.gizmo_handle = Some(handle);
        } else {
            self.gizmo_handle = Some(mvlc_core::GizmoHandle::Move);
        }
    }

    /// Stop dragging
    pub fn stop_drag(&mut self) {
        self.is_dragging = false;
        self.gizmo_handle = None;
    }
}

/// Main application state
pub struct AppState {
    pub project: Project,
    pub badges: Vec<RuntimeBadge>,
    pub audio_output: Option<AudioOutput>,
    pub canvas_viewport: CanvasViewport,
    pub canvas_interaction: CanvasInteraction,
    pub av_sync_manager: AvSyncManager<mvlc_media::AudioMasterClock>,
    pub renderer: Renderer,
    pub performance_monitor: PerformanceMonitor,
    pub color_pipeline: Option<ColorPipeline>,
}

impl AppState {
    pub fn new(window: &winit::window::Window) -> Self {
        let mut project = Project::new("Untitled Project".to_string(), 1920, 1080);

    // Create initial layers for multi-track testing
    let test_videos = vec![
        "test1.mp4".to_string(),
        "test2.mp4".to_string(),
        "test3.mp4".to_string(),
    ];

    project.layers.create_multi_track_setup(&test_videos);

        let badges = vec![
            RuntimeBadge::new("Decode".to_string(), "SW".to_string(), BadgeState::Fallback),
            RuntimeBadge::new("Transfer".to_string(), "Staged".to_string(), BadgeState::Partial),
            RuntimeBadge::new("Performance".to_string(), "0 B/frame".to_string(), BadgeState::Fallback),
            RuntimeBadge::new("Color".to_string(), "Basic".to_string(), BadgeState::Fallback),
            RuntimeBadge::new("Render".to_string(), "wgpu".to_string(), BadgeState::Partial),
            RuntimeBadge::new("Sync".to_string(), "A/V ±0ms".to_string(), BadgeState::Optimal),
        ];

        // Initialize audio output
        let audio_output = match AudioOutput::new_default() {
            Ok(audio) => {
                tracing::info!("Audio output initialized successfully");
                Some(audio)
            }
            Err(e) => {
                tracing::error!("Failed to initialize audio output: {}", e);
                None
            }
        };

        // Initialize A/V sync manager
        let mut av_sync_manager = AvSyncManager::new();

        // Add A/V sync streams for each layer
        if let Some(ref audio) = audio_output {
            for layer in project.layers.layers() {
                let stream_id = StreamId(layer.id.0 as u64);
                av_sync_manager.add_stream(stream_id, audio.master_clock().clone(), 30.0); // Assume 30fps for now
            }
        }

        // Initialize renderer - prefer Vulkan for DMA-BUF support
        let mut renderer = match Renderer::new_vulkan(&window) {
            Ok(r) => {
                tracing::info!("Using Vulkan renderer with DMA-BUF support");
                r
            }
            Err(e) => {
                tracing::warn!("Failed to create Vulkan renderer: {}, falling back to placeholder", e);
                Renderer::default()
            }
        };

        if let Err(e) = renderer.init() {
            tracing::warn!("Failed to initialize renderer: {}", e);
        }

        // Initialize color pipeline
        let color_pipeline = match ColorPipeline::new() {
            Ok(pipeline) => {
                tracing::info!("Color pipeline initialized successfully");
                Some(pipeline)
            }
            Err(e) => {
                tracing::warn!("Failed to initialize color pipeline: {}, color management will be unavailable", e);
                None
            }
        };

        Self {
            project,
            badges,
            audio_output,
            canvas_viewport: CanvasViewport::default(),
            canvas_interaction: CanvasInteraction {
                selected_layer: None,
                is_dragging: false,
                drag_start: [0.0, 0.0],
                gizmo_handle: None,
            },
            av_sync_manager,
            renderer,
            performance_monitor: PerformanceMonitor::new(),
            color_pipeline,
        }
    }
}

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
            .ok_or_else(|| anyhow::anyhow!("Failed to find a suitable GPU adapter"))?;

        let device_descriptor = wgpu::DeviceDescriptor {
            label: Some("mvlc-egui-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
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
            .find(|mode| matches!(mode, wgpu::CompositeAlphaMode::Opaque | wgpu::CompositeAlphaMode::Auto))
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
        let size = self.window.inner_size();
        ScreenDescriptor {
            size_in_pixels: [size.width.max(1), size.height.max(1)],
            pixels_per_point: self.window.scale_factor() as f32,
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Initialize GStreamer
    if let Err(e) = init_gstreamer() {
        tracing::warn!("Failed to initialize GStreamer: {}", e);
    } else {
        tracing::info!("GStreamer initialized successfully");
        let vaapi_available = check_vaapi_support();
        tracing::info!("VA-API hardware decoding available: {}", vaapi_available);
        let dmabuf_available = check_dmabuf_support();
        tracing::info!("DMA-BUF zero-copy memory available: {}", dmabuf_available);
    }

    tracing::info!("Starting MVLC application");

    let event_loop = EventLoop::new()?;
    let window = Arc::new(WindowBuilder::new()
        .with_title("MVLC - Modern Video Layered Compositor")
        .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
        .build(&event_loop)?);

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
                        // Egui rendering
                        let raw_input = egui_winit.take_egui_input(window.as_ref());
                        let full_output = egui_winit.egui_ctx().run(raw_input, |ctx| {
                            show_ui(ctx, &mut app_state);
                        });

                        egui_winit.handle_platform_output(window.as_ref(), full_output.platform_output);

                        let screen_descriptor = graphics_state.screen_descriptor();
                        let paint_jobs = egui_winit
                            .egui_ctx()
                            .tessellate(full_output.shapes.clone(), full_output.pixels_per_point);

                        let mut encoder = graphics_state
                            .device
                            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                label: Some("egui-wgpu encoder"),
                            });

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
                            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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

                        // Request redraw if needed
                        window.request_redraw();
                    }
                    _ => {}
                }
            }
            Event::AboutToWait => {
                // Request redraw to keep the UI updating
                window.request_redraw();
            }
            _ => {}
        }
    })?;

    Ok(())
}

fn handle_dropped_file(app_state: &mut AppState, path: &std::path::Path) {
    if let Some(extension) = path.extension() {
        let ext_str = extension.to_string_lossy().to_lowercase();
        if matches!(ext_str.as_str(), "mp4" | "mkv" | "avi" | "mov" | "webm" | "m4v") {
            tracing::info!("Video file detected: {}", path.display());

            // Create a new layer with the video file
            let layer_name = path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            let layer_id = app_state.project.layers.add_layer(layer_name.clone());

            // Position new layers slightly offset from each other
            let layer_count = app_state.project.layers.layers().len();

            // Set up the layer with the file
            if let Some(layer) = app_state.project.layers.get_layer_mut(layer_id) {
                layer.media_path = Some(path.to_string_lossy().to_string());
                layer.set_position((layer_count as f32 - 1.0) * 50.0, (layer_count as f32 - 1.0) * 30.0);
                layer.set_scale(0.8, 0.8); // Slightly smaller for multiple layers
            }

            // Add A/V sync stream for the new layer
            if let Some(ref audio) = app_state.audio_output {
                let stream_id = StreamId(layer_id.0 as u64);
                app_state.av_sync_manager.add_stream(stream_id, audio.master_clock().clone(), 30.0);
            }

            // Update badges to show file loading capability
            update_badges_for_state(app_state);

            tracing::info!("Created layer '{}' with video file", layer_name);
        } else {
            tracing::warn!("Unsupported file type: {} (extension: {})", path.display(), ext_str);
        }
    } else {
        tracing::warn!("File has no extension: {}", path.display());
    }
}

fn update_badges_for_state(app_state: &mut AppState) {
    // Update Decode badge based on loaded files and hardware availability
    let has_video_files = app_state.project.layers.layers()
        .iter()
        .any(|layer| layer.media_path.is_some());

    let vaapi_available = check_vaapi_support();

    if let Some(decode_badge) = app_state.badges.iter_mut().find(|b| b.name == "Decode") {
        if has_video_files {
            if vaapi_available {
                decode_badge.value = "VA-API (Ready)".to_string();
                decode_badge.state = BadgeState::Optimal;
            } else {
                decode_badge.value = "SW (File Ready)".to_string();
                decode_badge.state = BadgeState::Partial;
            }
        } else {
            if vaapi_available {
                decode_badge.value = "VA-API".to_string();
                decode_badge.state = BadgeState::Optimal;
            } else {
                decode_badge.value = "SW".to_string();
                decode_badge.state = BadgeState::Fallback;
            }
        }
    }

    // Update Sync badge based on A/V sync performance
    if let Some(sync_badge) = app_state.badges.iter_mut().find(|b| b.name == "Sync") {
        let overall_stats = app_state.av_sync_manager.overall_stats();

        if overall_stats.frames_presented > 0 {
            let avg_drift = overall_stats.avg_drift_ms;
            if avg_drift.abs() < 2.0 {
                sync_badge.state = BadgeState::Optimal;
                sync_badge.value = format!("A/V ±{:.1}ms", avg_drift);
            } else if avg_drift.abs() < 8.0 {
                sync_badge.state = BadgeState::Partial;
                sync_badge.value = format!("A/V ±{:.1}ms", avg_drift);
            } else {
                sync_badge.state = BadgeState::Fallback;
                sync_badge.value = format!("A/V ±{:.1}ms", avg_drift);
            }
        } else {
            sync_badge.value = "A/V ±0ms".to_string();
            sync_badge.state = BadgeState::Optimal;
        }
    }

    // Transfer badge - DMA-BUF zero-copy vs staged transfer
    let dmabuf_available = check_dmabuf_support();
    if let Some(transfer_badge) = app_state.badges.iter_mut().find(|b| b.name == "Transfer") {
        if dmabuf_available {
            transfer_badge.value = "Zero-Copy(DMA-BUF)".to_string();
            transfer_badge.state = BadgeState::Optimal; // DMA-BUF = zero-copy
        } else {
            transfer_badge.value = "Staged(Host->GPU)".to_string();
            transfer_badge.state = BadgeState::Partial; // Traditional upload
        }
    }

    // Performance badge - upload bytes monitoring
    let global_perf = app_state.performance_monitor.global_summary();
    if let Some(perf_badge) = app_state.badges.iter_mut().find(|b| b.name == "Performance") {
        perf_badge.value = global_perf.format_upload_bytes();

        if global_perf.is_fully_zero_copy && global_perf.avg_upload_bytes_per_frame < 1024.0 {
            perf_badge.state = BadgeState::Optimal; // True zero-copy
        } else if global_perf.zero_copy_streams > 0 {
            perf_badge.state = BadgeState::Partial; // Partial zero-copy
        } else {
            perf_badge.state = BadgeState::Fallback; // Traditional upload
        }
    }

    // Render badge - Vulkan renderer with DMA-BUF support
    if let Some(render_badge) = app_state.badges.iter_mut().find(|b| b.name == "Render") {
        if app_state.renderer.is_ready() {
            render_badge.value = "Vulkan(DMA-BUF)".to_string();
            render_badge.state = BadgeState::Optimal; // Vulkan with DMA-BUF support
        } else {
            render_badge.value = "Placeholder".to_string();
            render_badge.state = BadgeState::Partial;
        }
    }

    // Color badge - libplacebo HDR/SDR color management
    if let Some(color_badge) = app_state.badges.iter_mut().find(|b| b.name == "Color") {
        if let Some(color_pipeline) = &app_state.color_pipeline {
            let status = color_pipeline.status();
            color_badge.value = status.status_string();
            if status.is_hdr_enabled {
                color_badge.state = BadgeState::Optimal; // HDR support active
            } else {
                color_badge.state = BadgeState::Partial; // SDR with libplacebo
            }
        } else {
            color_badge.value = "Basic".to_string();
            color_badge.state = BadgeState::Fallback; // No libplacebo
        }
    }
}

fn show_ui(ctx: &egui::Context, app_state: &mut AppState) {
    // Update badges based on current state
    update_badges_for_state(app_state);
    // Top toolbar with badges
    egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.heading("MVLC - Runtime Status:");

            for badge in &app_state.badges {
                let color = badge.state.color();
                let text = badge.state.text();

                ui.colored_label(color, format!("{} {}: {}", text, badge.name, badge.value));
                ui.separator();
            }
        });
    });

    // Main canvas area (placeholder)
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading("Video Canvas");

        // Canvas area with interaction
        let canvas_response = ui.allocate_rect(ui.available_rect_before_wrap(), egui::Sense::click_and_drag());

        // Handle mouse events
        if canvas_response.clicked() {
            let canvas_pos = app_state.canvas_viewport.screen_to_canvas(
                [canvas_response.interact_pointer_pos().unwrap_or_default().x,
                 canvas_response.interact_pointer_pos().unwrap_or_default().y],
                canvas_response.rect
            );

            // Find clicked layer (back to front)
            let mut clicked_layer = None;
            let mut clicked_handle = None;

            for layer in app_state.project.layers.layers().iter().rev() {
                if let Some(handle) = app_state.canvas_interaction.hit_test_handle(canvas_pos, layer, 20.0) {
                    clicked_handle = Some((layer.id, handle));
                    break;
                } else if app_state.canvas_interaction.hit_test(canvas_pos, layer, &app_state.canvas_viewport) {
                    clicked_layer = Some(layer.id);
                    break;
                }
            }

            if let Some((layer_id, _handle)) = clicked_handle {
                app_state.canvas_interaction.selected_layer = Some(layer_id);
                // Get the layer for gizmo interaction
                if let Some(layer) = app_state.project.layers.get_layer(layer_id) {
                    app_state.canvas_interaction.start_drag(canvas_pos, layer, 20.0);
                }
            } else if let Some(layer_id) = clicked_layer {
                app_state.canvas_interaction.selected_layer = Some(layer_id);
                // Get the layer for gizmo interaction
                if let Some(layer) = app_state.project.layers.get_layer(layer_id) {
                    app_state.canvas_interaction.start_drag(canvas_pos, layer, 20.0);
                }
            } else {
                app_state.canvas_interaction.selected_layer = None;
                app_state.canvas_interaction.stop_drag();
            }
        }

        if canvas_response.drag_started() {
            // Start dragging if we have a selection
            if let Some(layer_id) = app_state.canvas_interaction.selected_layer {
                let canvas_pos = app_state.canvas_viewport.screen_to_canvas(
                    [canvas_response.interact_pointer_pos().unwrap_or_default().x,
                     canvas_response.interact_pointer_pos().unwrap_or_default().y],
                    canvas_response.rect
                );
                if let Some(layer) = app_state.project.layers.get_layer(layer_id) {
                    app_state.canvas_interaction.start_drag(canvas_pos, layer, 20.0);
                }
            }
        }

        if canvas_response.dragged() && app_state.canvas_interaction.is_dragging {
            let current_pos = app_state.canvas_viewport.screen_to_canvas(
                [canvas_response.interact_pointer_pos().unwrap_or_default().x,
                 canvas_response.interact_pointer_pos().unwrap_or_default().y],
                canvas_response.rect
            );

            let delta = [
                current_pos[0] - app_state.canvas_interaction.drag_start[0],
                current_pos[1] - app_state.canvas_interaction.drag_start[1],
            ];

            if let Some(layer_id) = app_state.canvas_interaction.selected_layer {
                if let Some(layer) = app_state.project.layers.get_layer_mut(layer_id) {
                    if let Some(handle) = app_state.canvas_interaction.gizmo_handle {
                        match handle {
                            mvlc_core::GizmoHandle::ScaleTopLeft => {
                                layer.transform.scale[0] *= (1.0 - delta[0] / 100.0).max(0.1);
                                layer.transform.scale[1] *= (1.0 - delta[1] / 100.0).max(0.1);
                            }
                            mvlc_core::GizmoHandle::ScaleTopRight => {
                                layer.transform.scale[0] *= (1.0 + delta[0] / 100.0).max(0.1);
                                layer.transform.scale[1] *= (1.0 - delta[1] / 100.0).max(0.1);
                            }
                            mvlc_core::GizmoHandle::ScaleBottomLeft => {
                                layer.transform.scale[0] *= (1.0 - delta[0] / 100.0).max(0.1);
                                layer.transform.scale[1] *= (1.0 + delta[1] / 100.0).max(0.1);
                            }
                            mvlc_core::GizmoHandle::ScaleBottomRight => {
                                layer.transform.scale[0] *= (1.0 + delta[0] / 100.0).max(0.1);
                                layer.transform.scale[1] *= (1.0 + delta[1] / 100.0).max(0.1);
                            }
                            mvlc_core::GizmoHandle::Rotate => {
                                layer.transform.rotation += delta[0] * 0.01;
                            }
                            mvlc_core::GizmoHandle::Move => {
                                // Move the layer
                                layer.transform.translation[0] += delta[0];
                                layer.transform.translation[1] += delta[1];
                            }
                            mvlc_core::GizmoHandle::None => {}
                        }
                    } else {
                        // Default to move if no gizmo handle
                        layer.transform.translation[0] += delta[0];
                        layer.transform.translation[1] += delta[1];
                    }
                }
            }

            app_state.canvas_interaction.drag_start = current_pos;
        }

        if !canvas_response.dragged() && canvas_response.drag_started() {
            // Handle drag release - this is a simplified approach
            // In a real implementation, we'd track drag state more carefully
        }

        // Stop dragging when mouse button is released
        if !canvas_response.is_pointer_button_down_on() && app_state.canvas_interaction.is_dragging {
            app_state.canvas_interaction.stop_drag();
        }

        // Draw canvas content
        let painter = ui.painter();
        let canvas_rect = canvas_response.rect;

        // Draw canvas background
        painter.rect_filled(
            canvas_rect,
            0.0,
            egui::Color32::from_rgb(40, 40, 40)
        );

        // Draw layers
        for layer in app_state.project.layers.render_order() {
            if !layer.visible {
                continue;
            }

            let layer_pos = app_state.canvas_viewport.canvas_to_screen(
                layer.transform.translation,
                canvas_rect
            );

            let layer_size = [
                400.0 * layer.transform.scale[0] * app_state.canvas_viewport.zoom,
                225.0 * layer.transform.scale[1] * app_state.canvas_viewport.zoom,
            ];

            let layer_rect = egui::Rect::from_center_size(
                egui::pos2(layer_pos[0], layer_pos[1]),
                egui::vec2(layer_size[0], layer_size[1])
            );

            // Skip if not visible
            if !canvas_rect.intersects(layer_rect) {
                continue;
            }

            // Draw layer background
            let layer_color = if Some(layer.id) == app_state.canvas_interaction.selected_layer {
                egui::Color32::from_rgb(100, 150, 200)
            } else {
                egui::Color32::from_rgb(80, 80, 80)
            };

            painter.rect_filled(layer_rect, 2.0, layer_color);

            // Draw layer border
            painter.rect_stroke(
                layer_rect,
                2.0,
                egui::Stroke::new(2.0, egui::Color32::WHITE)
            );

            // Draw layer name
            painter.text(
                layer_rect.center(),
                egui::Align2::CENTER_CENTER,
                &layer.name,
                egui::FontId::default(),
                egui::Color32::WHITE,
            );

            // Draw transform handles if selected
            if Some(layer.id) == app_state.canvas_interaction.selected_layer {
                let handle_color = egui::Color32::from_rgb(255, 255, 0);
                let handle_size = 8.0;

                // Corner handles
                let corners = [
                    [-layer_size[0]/2.0, -layer_size[1]/2.0],
                    [layer_size[0]/2.0, -layer_size[1]/2.0],
                    [-layer_size[0]/2.0, layer_size[1]/2.0],
                    [layer_size[0]/2.0, layer_size[1]/2.0],
                ];

                for corner in corners.iter() {
                    let handle_pos = [
                        layer_pos[0] + corner[0],
                        layer_pos[1] + corner[1],
                    ];

                    painter.circle_filled(
                        egui::pos2(handle_pos[0], handle_pos[1]),
                        handle_size,
                        handle_color,
                    );
                }

                // Rotate handle
                let rotate_pos = [
                    layer_pos[0],
                    layer_pos[1] - layer_size[1]/2.0 - 20.0,
                ];

                painter.circle_filled(
                    egui::pos2(rotate_pos[0], rotate_pos[1]),
                    handle_size,
                    handle_color,
                );

                // Draw rotation indicator line
                painter.line_segment(
                    [egui::pos2(layer_pos[0], layer_pos[1] - layer_size[1]/2.0),
                     egui::pos2(rotate_pos[0], rotate_pos[1])],
                    egui::Stroke::new(1.0, handle_color),
                );
            }
        }

        // Draw canvas info
        painter.text(
            canvas_rect.left_top() + egui::vec2(10.0, 10.0),
            egui::Align2::LEFT_TOP,
            format!("Zoom: {:.1}% | Offset: ({:.0}, {:.0})",
                app_state.canvas_viewport.zoom * 100.0,
                app_state.canvas_viewport.offset[0],
                app_state.canvas_viewport.offset[1]),
            egui::FontId::default(),
            egui::Color32::WHITE,
        );

        ui.separator();

        ui.heading("Playback Controls");
        ui.horizontal(|ui| {
            let play_button_text = if app_state.project.playing { "⏸" } else { "▶" };
            if ui.button(play_button_text).clicked() {
                if app_state.project.playing {
                    app_state.project.pause();
                } else {
                    app_state.project.play();
                }
            }

            if ui.button("⏹").clicked() {
                app_state.project.pause();
                app_state.project.seek(Time::ZERO);
            }

            if ui.button("⏮").clicked() {
                let current_time = app_state.project.playback_position;
                let new_time = Time::from_secs((current_time.as_secs() as i64 - 10).max(0));
                app_state.project.seek(new_time);
            }

            if ui.button("⏭").clicked() {
                let current_time = app_state.project.playback_position;
                let new_time = Time::from_secs(current_time.as_secs() as i64 + 10);
                app_state.project.seek(new_time);
            }
        });

        // Timeline
        let duration = Time::from_secs(60); // Placeholder duration
        let progress = app_state.project.playback_position.as_millis() as f64 / duration.as_millis() as f64;
        let mut progress_normalized = progress.min(1.0);

        ui.horizontal(|ui| {
            ui.label("Timeline:");
            if ui.add(egui::Slider::new(&mut progress_normalized, 0.0..=1.0)).changed() {
                let new_time = Time::from_millis((progress_normalized * duration.as_millis() as f64) as i64);
                app_state.project.seek(new_time);
            }
            ui.label(format!("{:.1}s / {:.1}s",
                app_state.project.playback_position.as_millis() as f64 / 1000.0,
                duration.as_millis() as f64 / 1000.0));
        });

        ui.separator();

        ui.heading("Project Info");
        ui.label(format!("Project: {}", app_state.project.metadata.name));
        ui.label(format!("Layers: {}", app_state.project.layers.layers().len()));
        ui.label(format!("Status: {}", if app_state.project.playing { "Playing" } else { "Paused" }));

        ui.separator();

        ui.heading("Audio System");
        if let Some(audio) = &app_state.audio_output {
            let current_time = audio.current_time();
            ui.label(format!("Audio Time: {:.3}s", current_time.as_millis() as f64 / 1000.0));
            ui.label(format!("Sample Rate: {} Hz", audio.config().sample_rate));
            ui.label(format!("Channels: {}", audio.config().channels));

            ui.separator();

            ui.heading("A/V Sync");
            let overall_stats = app_state.av_sync_manager.overall_stats();
            ui.label(format!("Frames Presented: {}", overall_stats.frames_presented));
            ui.label(format!("Frames Dropped: {}", overall_stats.frames_dropped));
            ui.label(format!("Frames Repeated: {}", overall_stats.frames_repeated));
            ui.label(format!("Avg Drift: {:.2}ms", overall_stats.avg_drift_ms));
            ui.label(format!("Max Drift: {:.2}ms", overall_stats.max_drift_ms));

            // Show per-layer sync stats
            for layer in app_state.project.layers.layers() {
                let stream_id = StreamId(layer.id.0 as u64);
                if let Some(stats) = app_state.av_sync_manager.stats(&stream_id) {
                    ui.collapsing(format!("Layer {} Sync", layer.name), |ui| {
                        ui.label(format!("Presented: {}", stats.frames_presented));
                        ui.label(format!("Dropped: {}", stats.frames_dropped));
                        ui.label(format!("Repeated: {}", stats.frames_repeated));
                    });
                }
            }
        } else {
            ui.colored_label(egui::Color32::RED, "Audio: Not initialized");
        }

        ui.separator();

        // Canvas controls
        ui.horizontal(|ui| {
            ui.label("Canvas:");
            if ui.button("🔍+").clicked() {
                app_state.canvas_viewport.zoom *= 1.2;
            }
            if ui.button("🔍-").clicked() {
                app_state.canvas_viewport.zoom /= 1.2;
                app_state.canvas_viewport.zoom = app_state.canvas_viewport.zoom.max(0.1);
            }
            if ui.button("🏠").clicked() {
                app_state.canvas_viewport.zoom = 1.0;
                app_state.canvas_viewport.offset = [0.0, 0.0];
            }
        });

        ui.separator();

        ui.heading("Layers");

        // Display layers (read-only for now to avoid borrowing issues)
        for layer in app_state.project.layers.layers() {
            ui.horizontal(|ui| {
                ui.label(if layer.visible { "👁" } else { "🙈" });
                ui.label(format!("{} (Z: {})", layer.name, layer.z_index));

                if Some(layer.id) == app_state.canvas_interaction.selected_layer {
                    ui.colored_label(egui::Color32::YELLOW, "●");
                }

                if layer.playing {
                    ui.colored_label(egui::Color32::GREEN, "▶");
                } else {
                    ui.label("⏸");
                }

                ui.label(format!("Pos: {:.0},{:.0}", layer.transform.translation[0], layer.transform.translation[1]));
                ui.label(format!("Scale: {:.2},{:.2}", layer.transform.scale[0], layer.transform.scale[1]));
            });
        }

        ui.horizontal(|ui| {
            if ui.button("➕ Add Layer").clicked() {
                let layer_name = format!("Layer {}", app_state.project.layers.layers().len() + 1);
                let _layer_id = app_state.project.layers.add_layer(layer_name);
            }

            if ui.button("📁 Load Video").clicked() {
                // For now, just create a placeholder layer
                // In a real implementation, this would open a file dialog
                let layer_count = app_state.project.layers.layers().len() + 1;
                let layer_name = format!("Video Layer {}", layer_count);
                let layer_id = app_state.project.layers.add_layer(layer_name.clone());

                if let Some(layer) = app_state.project.layers.get_layer_mut(layer_id) {
                    layer.media_path = Some("placeholder.mp4".to_string()); // Placeholder
                    layer.set_position((layer_count as f32 - 1.0) * 50.0, (layer_count as f32 - 1.0) * 30.0);
                    layer.set_scale(0.8, 0.8);
                }

                if let Some(ref audio) = app_state.audio_output {
                    let stream_id = StreamId(layer_id.0 as u64);
                    app_state.av_sync_manager.add_stream(stream_id, audio.master_clock().clone(), 30.0);
                }

                update_badges_for_state(app_state);

                tracing::info!("Created placeholder layer '{}'", layer_name);
            }
        });

        ui.label("💡 Drag & drop video files (.mp4, .mkv, .avi, .mov, .webm) onto the window to add them as layers");
    });
}
