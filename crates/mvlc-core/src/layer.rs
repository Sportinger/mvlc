//! Layer types for video composition
//!
//! Layers represent individual video streams in the composition canvas.
//! Each layer has its own transform, opacity, and other properties.

use serde::{Deserialize, Serialize};
use crate::time::Time;

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
}

/// Collection of layers with ordering
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerStack {
    layers: Vec<Layer>,
    next_id: LayerId,
}

impl LayerStack {
    pub fn new() -> Self {
        Self {
            layers: Vec::new(),
            next_id: LayerId(1),
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
}

impl Default for LayerStack {
    fn default() -> Self {
        Self::new()
    }
}
