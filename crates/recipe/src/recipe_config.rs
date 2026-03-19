use crate::live_display::LiveDisplay;

/// Stores recipe-wide configuration settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeConfig {
    /// Chooses the interactive live display mode for task execution.
    pub live_display: LiveDisplay,
    /// Overrides how many task processes may run at the same time.
    pub max_parallel_tasks: Option<usize>,
}

impl Default for RecipeConfig {
    fn default() -> Self {
        Self {
            live_display: LiveDisplay::LinePerTask,
            max_parallel_tasks: None,
        }
    }
}
