use crate::live_display::LiveDisplay;

/// Stores recipe-wide configuration settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeConfig {
    /// Chooses the interactive live display mode for task execution.
    pub live_display: LiveDisplay,
}

impl Default for RecipeConfig {
    fn default() -> Self {
        Self {
            live_display: LiveDisplay::LinePerTask,
        }
    }
}
