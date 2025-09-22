use egui::Color32;

/// Runtime badge states for visual path transparency
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BadgeState {
    Optimal,
    Partial,
    Fallback,
}

impl BadgeState {
    pub fn color(&self) -> Color32 {
        match self {
            BadgeState::Optimal => Color32::from_rgb(34, 197, 94), // Green
            BadgeState::Partial => Color32::from_rgb(251, 191, 36), // Yellow
            BadgeState::Fallback => Color32::from_rgb(239, 68, 68), // Red
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

/// Runtime badge information displayed in the UI toolbar
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
