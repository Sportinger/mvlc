#[derive(Debug, Clone)]
pub struct TransportState {
    pub is_playing: bool,
    pub position_seconds: f32,
    pub duration_seconds: f32,
}

impl Default for TransportState {
    fn default() -> Self {
        Self {
            is_playing: false,
            position_seconds: 0.0,
            duration_seconds: 0.0,
        }
    }
}

pub fn format_timecode(seconds: f32) -> String {
    let total_seconds = seconds.max(0.0).round() as u32;
    let minutes = total_seconds / 60;
    let secs = total_seconds % 60;
    format!("{:02}:{:02}", minutes, secs)
}
