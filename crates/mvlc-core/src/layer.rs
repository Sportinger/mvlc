//! Layer types for video composition
//!
//! Layers represent individual video streams in the composition canvas.
//! Each layer has its own transform, opacity, and other properties.

use serde::{Deserialize, Serialize};
use crate::{time::Time, StreamId};

/// Unique identifier for a layer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LayerId(pub u64);

impl LayerId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    pub fn next(&self) -> Self {
        Self(self.0 + 1)
    }
}

/// 2D transformation matrix for layer positioning and scaling
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub translation: [f32; 2],
    pub scale: [f32; 2],
    pub rotation: f32, // in radians
}

impl Transform {
    pub fn identity() -> Self {
        Self {
            translation: [0.0, 0.0],
            scale: [1.0, 1.0],
            rotation: 0.0,
        }
    }

    pub fn translate(mut self, x: f32, y: f32) -> Self {
        self.translation[0] += x;
        self.translation[1] += y;
        self
    }

    pub fn scale(mut self, sx: f32, sy: f32) -> Self {
        self.scale[0] *= sx;
        self.scale[1] *= sy;
        self
    }

    pub fn rotate(mut self, radians: f32) -> Self {
        self.rotation += radians;
        self
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::identity()
    }
}

/// Gizmo handle types for layer manipulation
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GizmoHandle {
    None,
    Move,           // Move the entire layer
    ScaleTopLeft,
    ScaleTopRight,
    ScaleBottomLeft,
    ScaleBottomRight,
    Rotate,         // Rotate handle
}

/// Layer properties for video composition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub transform: Transform,
    pub opacity: f32,
    pub visible: bool,
    pub z_index: i32,

    // Media properties
    pub media_path: Option<String>,
    pub start_time: Time,
    pub duration: Option<Time>,
    pub loop_enabled: bool,

    // Playback state
    pub playing: bool,
    pub muted: bool,

    // Stream association (for multi-track compositing)
    pub stream_id: Option<StreamId>,

    // Gizmo state (for canvas interaction)
    pub selected: bool,
    pub gizmo_visible: bool,
}

impl Layer {
    pub fn new(id: LayerId, name: String) -> Self {
        Self {
            id,
            name,
            transform: Transform::identity(),
            opacity: 1.0,
            visible: true,
            z_index: 0,
            media_path: None,
            start_time: Time::ZERO,
            duration: None,
            loop_enabled: false,
            playing: false,
            muted: false,
            stream_id: None,
            selected: false,
            gizmo_visible: false,
        }
    }

    pub fn with_media(mut self, path: String) -> Self {
        self.media_path = Some(path);
        self
    }

    pub fn set_position(&mut self, x: f32, y: f32) {
        self.transform.translation = [x, y];
    }

    pub fn set_scale(&mut self, sx: f32, sy: f32) {
        self.transform.scale = [sx, sy];
    }

    pub fn set_rotation(&mut self, radians: f32) {
        self.transform.rotation = radians;
    }

    pub fn set_opacity(&mut self, opacity: f32) {
        self.opacity = opacity.clamp(0.0, 1.0);
    }

    pub fn set_z_index(&mut self, z: i32) {
        self.z_index = z;
    }

    pub fn play(&mut self) {
        self.playing = true;
    }

    pub fn pause(&mut self) {
        self.playing = false;
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
    }

    /// Associate this layer with a video stream
    pub fn set_stream(&mut self, stream_id: StreamId) {
        self.stream_id = Some(stream_id);
    }

    /// Remove stream association
    pub fn clear_stream(&mut self) {
        self.stream_id = None;
    }

    /// Select this layer for manipulation
    pub fn select(&mut self) {
        self.selected = true;
        self.gizmo_visible = true;
    }

    /// Deselect this layer
    pub fn deselect(&mut self) {
        self.selected = false;
        self.gizmo_visible = false;
    }

    /// Toggle selection state
    pub fn toggle_selection(&mut self) {
        if self.selected {
            self.deselect();
        } else {
            self.select();
        }
    }

    /// Show/hide gizmos for this layer
    pub fn set_gizmo_visible(&mut self, visible: bool) {
        self.gizmo_visible = visible;
    }

    /// Get the layer's bounding box in world space
    pub fn bounds(&self, width: f32, height: f32) -> ([f32; 2], [f32; 2]) {
        let hw = width * self.transform.scale[0] * 0.5;
        let hh = height * self.transform.scale[1] * 0.5;

        let cos = self.transform.rotation.cos();
        let sin = self.transform.rotation.sin();

        // Calculate corners
        let corners = [
            [-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]
        ];

        let mut min_x = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;

        for [x, y] in corners {
            // Apply rotation
            let rx = x * cos - y * sin;
            let ry = x * sin + y * cos;

            // Apply translation
            let wx = rx + self.transform.translation[0];
            let wy = ry + self.transform.translation[1];

            min_x = min_x.min(wx);
            max_x = max_x.max(wx);
            min_y = min_y.min(wy);
            max_y = max_y.max(wy);
        }

        ([min_x, min_y], [max_x, max_y])
    }

    /// Check if a point is inside this layer's bounds
    pub fn contains_point(&self, point: [f32; 2], width: f32, height: f32) -> bool {
        let (min, max) = self.bounds(width, height);
        point[0] >= min[0] && point[0] <= max[0] &&
        point[1] >= min[1] && point[1] <= max[1]
    }

    /// Get gizmo handle at a specific point
    pub fn gizmo_handle_at(&self, point: [f32; 2], width: f32, height: f32, handle_size: f32) -> GizmoHandle {
        if !self.selected || !self.gizmo_visible {
            return GizmoHandle::None;
        }

        // Check rotation handle (top center)
        let center_x = self.transform.translation[0];
        let center_y = self.transform.translation[1];
        let handle_y = center_y - height * self.transform.scale[1] * 0.5 - handle_size;

        if (point[0] - center_x).abs() < handle_size &&
           (point[1] - handle_y).abs() < handle_size {
            return GizmoHandle::Rotate;
        }

        // Check scale handles
        let half_w = width * self.transform.scale[0] * 0.5;
        let half_h = height * self.transform.scale[1] * 0.5;

        let handles = [
            (center_x - half_w, center_y - half_h, GizmoHandle::ScaleTopLeft),
            (center_x + half_w, center_y - half_h, GizmoHandle::ScaleTopRight),
            (center_x - half_w, center_y + half_h, GizmoHandle::ScaleBottomLeft),
            (center_x + half_w, center_y + half_h, GizmoHandle::ScaleBottomRight),
        ];

        for (hx, hy, handle) in handles {
            if (point[0] - hx).abs() < handle_size &&
               (point[1] - hy).abs() < handle_size {
                return handle;
            }
        }

        GizmoHandle::None
    }
}

/// Collection of layers with ordering and multi-track support
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerStack {
    layers: Vec<Layer>,
    next_id: LayerId,
    selected_layer: Option<LayerId>,
}

impl LayerStack {
    pub fn new() -> Self {
        Self {
            layers: Vec::new(),
            next_id: LayerId(1),
            selected_layer: None,
        }
    }

    pub fn add_layer(&mut self, name: String) -> LayerId {
        let id = self.next_id;
        self.next_id = self.next_id.next();

        let layer = Layer::new(id, name);
        self.layers.push(layer);

        // Sort by z-index
        self.layers.sort_by_key(|l| l.z_index);

        id
    }

    pub fn remove_layer(&mut self, id: LayerId) -> bool {
        if let Some(pos) = self.layers.iter().position(|l| l.id == id) {
            self.layers.remove(pos);
            true
        } else {
            false
        }
    }

    pub fn get_layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    pub fn get_layer_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    pub fn layers_mut(&mut self) -> &mut [Layer] {
        &mut self.layers
    }

    pub fn reorder_layer(&mut self, id: LayerId, new_z: i32) {
        if let Some(layer) = self.get_layer_mut(id) {
            layer.z_index = new_z;
        }
        self.layers.sort_by_key(|l| l.z_index);
    }

    /// Get layers in rendering order (back to front, low z-index to high)
    pub fn render_order(&self) -> impl Iterator<Item = &Layer> {
        self.layers.iter()
    }

    /// Get layers in reverse rendering order (front to back)
    pub fn reverse_render_order(&self) -> impl Iterator<Item = &Layer> {
        self.layers.iter().rev()
    }

    /// Get the currently selected layer
    pub fn selected_layer(&self) -> Option<&Layer> {
        self.selected_layer.and_then(|id| self.get_layer(id))
    }

    /// Get the currently selected layer mutably
    pub fn selected_layer_mut(&mut self) -> Option<&mut Layer> {
        if let Some(id) = self.selected_layer {
            self.get_layer_mut(id)
        } else {
            None
        }
    }

    /// Select a layer by ID
    pub fn select_layer(&mut self, id: LayerId) -> bool {
        if self.get_layer(id).is_some() {
            // Deselect current layer
            if let Some(current_id) = self.selected_layer {
                if let Some(layer) = self.get_layer_mut(current_id) {
                    layer.deselect();
                }
            }

            // Select new layer
            if let Some(layer) = self.get_layer_mut(id) {
                layer.select();
            }

            self.selected_layer = Some(id);
            true
        } else {
            false
        }
    }

    /// Deselect the current layer
    pub fn deselect_layer(&mut self) {
        if let Some(id) = self.selected_layer {
            if let Some(layer) = self.get_layer_mut(id) {
                layer.deselect();
            }
        }
        self.selected_layer = None;
    }

    /// Get all layers with active video streams
    pub fn active_video_layers(&self) -> impl Iterator<Item = &Layer> {
        self.layers.iter().filter(|layer| layer.stream_id.is_some())
    }

    /// Get all active stream IDs
    pub fn active_streams(&self) -> impl Iterator<Item = StreamId> + '_ {
        self.active_video_layers().filter_map(|layer| layer.stream_id)
    }

    /// Count layers with active streams
    pub fn active_stream_count(&self) -> usize {
        self.active_video_layers().count()
    }

    /// Get layer by stream ID
    pub fn get_layer_by_stream(&self, stream_id: StreamId) -> Option<&Layer> {
        self.layers.iter().find(|layer| layer.stream_id == Some(stream_id))
    }

    /// Get layer by stream ID mutably
    pub fn get_layer_by_stream_mut(&mut self, stream_id: StreamId) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|layer| layer.stream_id == Some(stream_id))
    }

    /// Associate a layer with a stream
    pub fn associate_stream(&mut self, layer_id: LayerId, stream_id: StreamId) -> bool {
        if let Some(layer) = self.get_layer_mut(layer_id) {
            layer.set_stream(stream_id);
            true
        } else {
            false
        }
    }

    /// Remove stream association from a layer
    pub fn disassociate_stream(&mut self, layer_id: LayerId) -> bool {
        if let Some(layer) = self.get_layer_mut(layer_id) {
            layer.clear_stream();
            true
        } else {
            false
        }
    }

    /// Find layer at canvas position
    pub fn layer_at_position(&self, position: [f32; 2], canvas_width: f32, canvas_height: f32) -> Option<&Layer> {
        // Check in reverse render order (front to back) for hit testing
        for layer in self.reverse_render_order() {
            if layer.visible && layer.contains_point(position, canvas_width, canvas_height) {
                return Some(layer);
            }
        }
        None
    }

    /// Find gizmo handle at canvas position
    pub fn gizmo_handle_at_position(&self, position: [f32; 2], canvas_width: f32, canvas_height: f32, handle_size: f32) -> (Option<LayerId>, GizmoHandle) {
        if let Some(layer) = self.selected_layer() {
            let handle = layer.gizmo_handle_at(position, canvas_width, canvas_height, handle_size);
            if handle != GizmoHandle::None {
                return (Some(layer.id), handle);
            }
        }
        (None, GizmoHandle::None)
    }

    /// Create a multi-layer composition with multiple video streams
    pub fn create_multi_track_setup(&mut self, video_paths: &[String]) -> Vec<LayerId> {
        let mut layer_ids = Vec::new();

        for (i, path) in video_paths.iter().enumerate() {
            let name = format!("Video {}", i + 1);
            let layer_id = self.add_layer(name);

            if let Some(layer) = self.get_layer_mut(layer_id) {
                layer.media_path = Some(path.clone());

                // Offset layers for visibility
                let offset = i as f32 * 50.0;
                layer.set_position(100.0 + offset, 100.0 + offset);
                layer.set_scale(0.8, 0.8);
                layer.opacity = 0.8;
            }

            layer_ids.push(layer_id);
        }

        layer_ids
    }
}

impl Default for LayerStack {
    fn default() -> Self {
        Self::new()
    }
}
