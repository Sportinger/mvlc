use egui::{self, Rect};
use mvlc_core::{Layer, LayerId};

/// Canvas viewport and interaction state
#[derive(Debug, Clone)]
pub struct CanvasViewport {
    pub offset: [f32; 2],
    pub zoom: f32,
    pub size: [f32; 2],
}

impl Default for CanvasViewport {
    fn default() -> Self {
        Self {
            offset: [0.0, 0.0],
            zoom: 1.0,
            size: [1920.0, 1080.0],
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
    pub fn screen_to_canvas(&self, screen_pos: [f32; 2], canvas_rect: Rect) -> [f32; 2] {
        let canvas_center = [canvas_rect.center().x, canvas_rect.center().y];

        [
            (screen_pos[0] - canvas_center[0]) / self.zoom + self.offset[0],
            (screen_pos[1] - canvas_center[1]) / self.zoom + self.offset[1],
        ]
    }

    /// Convert canvas coordinates to screen coordinates
    pub fn canvas_to_screen(&self, canvas_pos: [f32; 2], canvas_rect: Rect) -> [f32; 2] {
        let canvas_center = [canvas_rect.center().x, canvas_rect.center().y];

        [
            (canvas_pos[0] - self.offset[0]) * self.zoom + canvas_center[0],
            (canvas_pos[1] - self.offset[1]) * self.zoom + canvas_center[1],
        ]
    }

    /// Check if a canvas position is visible in the current viewport
    pub fn is_visible(&self, canvas_pos: [f32; 2], canvas_rect: Rect) -> bool {
        let screen_pos = self.canvas_to_screen(canvas_pos, canvas_rect);
        canvas_rect.contains(egui::pos2(screen_pos[0], screen_pos[1]))
    }
}

impl CanvasInteraction {
    /// Check if a point hits a layer
    pub fn hit_test(
        &self,
        canvas_pos: [f32; 2],
        layer: &Layer,
        _viewport: &CanvasViewport,
    ) -> bool {
        let video_width = 1920.0;
        let video_height = 1080.0;
        layer.contains_point(canvas_pos, video_width, video_height)
    }

    /// Find the gizmo handle at a given canvas position for a layer
    pub fn hit_test_handle(
        &self,
        canvas_pos: [f32; 2],
        layer: &Layer,
        handle_size: f32,
    ) -> Option<mvlc_core::GizmoHandle> {
        if !layer.visible || !layer.selected {
            return None;
        }

        let video_width = 1920.0;
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
