use mvlc_core::Project;
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

/// Main application state
pub struct AppState {
    pub project: Project,
    pub badges: Vec<RuntimeBadge>,
}

impl AppState {
    pub fn new() -> Self {
        let mut project = Project::new("Untitled Project".to_string(), 1920, 1080);

        // Add a test layer
        let layer_id = project.layers.add_layer("Test Video".to_string());
        if let Some(layer) = project.layers.get_layer_mut(layer_id) {
            layer.set_position(100.0, 100.0);
            layer.set_scale(0.5, 0.5);
        }

        let badges = vec![
            RuntimeBadge::new("Decode".to_string(), "SW".to_string(), BadgeState::Fallback),
            RuntimeBadge::new("Color".to_string(), "Basic".to_string(), BadgeState::Fallback),
            RuntimeBadge::new("Transfer".to_string(), "Staged".to_string(), BadgeState::Partial),
            RuntimeBadge::new("Render".to_string(), "wgpu".to_string(), BadgeState::Partial),
            RuntimeBadge::new("Sync".to_string(), "A/V ±0ms".to_string(), BadgeState::Optimal),
        ];

        Self { project, badges }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    tracing::info!("Starting MVLC application");

    let event_loop = EventLoop::new()?;
    let window = WindowBuilder::new()
        .with_title("MVLC - Modern Video Layered Compositor")
        .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
        .build(&event_loop)?;

    let mut egui_winit = egui_winit::State::new(
        egui::Context::default(),
        egui::ViewportId::default(),
        &window,
        None,
        None,
    );

    let app_state = AppState::new();

    event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::Poll);

        match event {
            Event::WindowEvent { event, .. } => {
                let response = egui_winit.on_window_event(&window, &event);
                if response.consumed {
                    return;
                }

                match event {
                    WindowEvent::CloseRequested => {
                        tracing::info!("Window close requested, exiting");
                        elwt.exit();
                    }
                    WindowEvent::RedrawRequested => {
                        // Egui rendering
                        let raw_input = egui_winit.take_egui_input(&window);
                        let full_output = egui_winit.egui_ctx().run(raw_input, |ctx| {
                            show_ui(ctx, &app_state);
                        });

                        egui_winit.handle_platform_output(&window, full_output.platform_output);

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

fn show_ui(ctx: &egui::Context, app_state: &AppState) {
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
        ui.label("Canvas rendering will be implemented here");

        ui.separator();

        ui.heading("Project Info");
        ui.label(format!("Project: {}", app_state.project.metadata.name));
        ui.label(format!("Layers: {}", app_state.project.layers.layers().len()));

        ui.separator();

        ui.heading("Layers");
        for layer in app_state.project.layers.layers() {
            ui.horizontal(|ui| {
                ui.label(if layer.visible { "👁" } else { "🙈" });
                ui.label(format!("{} (Z: {})", layer.name, layer.z_index));
                if layer.playing {
                    ui.colored_label(egui::Color32::GREEN, "▶");
                } else {
                    ui.label("⏸");
                }
                ui.label(format!("Pos: {:.0}x{:.0}", layer.transform.translation[0], layer.transform.translation[1]));
                ui.label(format!("Scale: {:.2}x{:.2}", layer.transform.scale[0], layer.transform.scale[1]));
            });
        }
    });
}
