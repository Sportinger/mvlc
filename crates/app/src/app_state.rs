use std::collections::HashMap;
use std::time::Duration;
use winit::window::Window;

use crate::badges::{BadgeState, RuntimeBadge};
use crate::canvas::{CanvasInteraction, CanvasViewport};
use crate::transport::TransportState;
use crate::video_layer::VideoLayerState;
use egui_wgpu::Renderer as EguiWgpuRenderer;
use mvlc_core::{AvSyncManager, LayerId, PerformanceMonitor, Project, StreamId};
use mvlc_media::{AudioMasterClock, AudioOutput};
use mvlc_render::{ColorPipeline, Renderer};

pub struct AppState {
    pub project: Project,
    pub badges: Vec<RuntimeBadge>,
    pub audio_output: Option<AudioOutput>,
    pub canvas_viewport: CanvasViewport,
    pub canvas_interaction: CanvasInteraction,
    pub av_sync_manager: AvSyncManager<AudioMasterClock>,
    pub renderer: Renderer,
    pub performance_monitor: PerformanceMonitor,
    pub color_pipeline: Option<ColorPipeline>,
    pub transport: TransportState,
    pub(crate) video_layers: HashMap<LayerId, VideoLayerState>,
}

impl AppState {
    pub fn new(window: &Window) -> Self {
        let project = Project::new("Untitled Project".to_string(), 1920, 1080);

        let badges = vec![
            RuntimeBadge::new("Decode".to_string(), "SW".to_string(), BadgeState::Fallback),
            RuntimeBadge::new(
                "Transfer".to_string(),
                "Staged".to_string(),
                BadgeState::Partial,
            ),
            RuntimeBadge::new(
                "Performance".to_string(),
                "0 B/frame".to_string(),
                BadgeState::Fallback,
            ),
            RuntimeBadge::new(
                "Color".to_string(),
                "Basic".to_string(),
                BadgeState::Fallback,
            ),
            RuntimeBadge::new(
                "Render".to_string(),
                "wgpu".to_string(),
                BadgeState::Partial,
            ),
            RuntimeBadge::new(
                "Sync".to_string(),
                "A/V ±0ms".to_string(),
                BadgeState::Optimal,
            ),
        ];

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

        let mut av_sync_manager = AvSyncManager::new();

        if let Some(ref audio) = audio_output {
            for layer in project.layers.layers() {
                let stream_id = StreamId(layer.id.0 as u64);
                av_sync_manager.add_stream(stream_id, audio.master_clock().clone(), 30.0);
            }
        }

        let use_vulkan = false;

        let mut renderer = if use_vulkan {
            match Renderer::new_vulkan(window) {
                Ok(r) => {
                    tracing::info!("Using Vulkan renderer with DMA-BUF support");
                    r
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to create Vulkan renderer: {}, falling back to placeholder",
                        e
                    );
                    Renderer::default()
                }
            }
        } else {
            Renderer::default()
        };

        if let Err(e) = renderer.init() {
            tracing::warn!("Failed to initialize renderer: {}", e);
        }

        let color_pipeline = match ColorPipeline::new() {
            Ok(pipeline) => {
                tracing::info!("Color pipeline initialized successfully");
                Some(pipeline)
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to initialize color pipeline: {}, color management will be unavailable",
                    e
                );
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
            transport: TransportState::default(),
            video_layers: HashMap::new(),
        }
    }

    /// Poll all active video decoders for fresh frames and update performance stats
    pub fn poll_video_frames(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut EguiWgpuRenderer,
    ) {
        for (_layer_id, state) in self.video_layers.iter_mut() {
            if let Some(frame) = state.update_with_latest_frame(device, queue, renderer) {
                let upload_bytes = frame.data_size_bytes() as u64;
                self.performance_monitor.record_frame(
                    frame.stream_id.0,
                    upload_bytes,
                    Duration::from_millis(16),
                    false,
                );
            }
        }
    }

    /// Drain video decoders without uploading frames. Useful for headless render paths.
    pub fn poll_video_frames_headless(&mut self) {
        for (_layer_id, state) in self.video_layers.iter_mut() {
            if let Some(frame) = state.drain_latest_frame() {
                self.performance_monitor.record_frame(
                    frame.stream_id.0,
                    0,
                    Duration::from_millis(16),
                    false,
                );
            }
        }
    }

    pub fn video_layers_mut(&mut self) -> &mut HashMap<LayerId, VideoLayerState> {
        &mut self.video_layers
    }
}
