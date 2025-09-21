//! Project management types
//!
//! Handles saving and loading project state, including layers,
//! settings, and metadata.

use serde::{Deserialize, Serialize};
use crate::layer::LayerStack;
use crate::time::Time;

/// Project metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMetadata {
    pub name: String,
    pub version: String,
    pub created_at: Time,
    pub modified_at: Time,
    pub canvas_width: u32,
    pub canvas_height: u32,
    pub frame_rate: f32,
}

impl ProjectMetadata {
    pub fn new(name: String, width: u32, height: u32) -> Self {
        let now = Time::ZERO; // TODO: Use actual system time
        Self {
            name,
            version: env!("CARGO_PKG_VERSION").to_string(),
            created_at: now,
            modified_at: now,
            canvas_width: width,
            canvas_height: height,
            frame_rate: 60.0, // Default to 60fps
        }
    }
}

/// Complete project state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub metadata: ProjectMetadata,
    pub layers: LayerStack,
    pub playback_position: Time,
    pub playing: bool,
}

impl Project {
    pub fn new(name: String, width: u32, height: u32) -> Self {
        Self {
            metadata: ProjectMetadata::new(name, width, height),
            layers: LayerStack::new(),
            playback_position: Time::ZERO,
            playing: false,
        }
    }

    pub fn play(&mut self) {
        self.playing = true;
    }

    pub fn pause(&mut self) {
        self.playing = false;
    }

    pub fn seek(&mut self, position: Time) {
        self.playback_position = position;
    }

    pub fn update_modified_time(&mut self) {
        self.metadata.modified_at = Time::ZERO; // TODO: Use actual system time
    }
}

/// Project serialization/deserialization
pub mod io {
    use super::*;
    use std::path::Path;
    use std::fs;
    use thiserror::Error;

    #[derive(Error, Debug)]
    pub enum ProjectError {
        #[error("IO error: {0}")]
        Io(#[from] std::io::Error),
        #[error("Serialization error: {0}")]
        Serde(#[from] serde_json::Error),
        #[error("Invalid project format")]
        InvalidFormat,
    }

    pub type Result<T> = std::result::Result<T, ProjectError>;

    /// Save project to JSON file
    pub fn save_project(project: &Project, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(project)?;
        fs::write(path, json)?;
        Ok(())
    }

    /// Load project from JSON file
    pub fn load_project(path: &Path) -> Result<Project> {
        let json = fs::read_to_string(path)?;
        let project: Project = serde_json::from_str(&json)?;
        Ok(project)
    }

    /// Save project to RON format (more human-readable than JSON)
    pub fn save_project_ron(project: &Project, path: &Path) -> Result<()> {
        let ron = ron::to_string(project).map_err(|_| ProjectError::InvalidFormat)?;
        fs::write(path, ron)?;
        Ok(())
    }

    /// Load project from RON format
    pub fn load_project_ron(path: &Path) -> Result<Project> {
        let ron = fs::read_to_string(path)?;
        let project: Project = ron::from_str(&ron).map_err(|_| ProjectError::InvalidFormat)?;
        Ok(project)
    }
}
