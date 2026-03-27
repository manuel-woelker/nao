use crate::failure_mode::FailureMode;
use crate::live_display::LiveDisplay;

/// Stores recipe-wide configuration settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeConfig {
    /// Chooses the interactive live display mode for task execution.
    pub live_display: LiveDisplay,
    /// Chooses whether failures stop scheduling immediately or allow unrelated work to continue.
    pub failure_mode: FailureMode,
    /// Overrides how many task processes may run at the same time.
    pub max_parallel_tasks: Option<usize>,
}

impl Default for RecipeConfig {
    fn default() -> Self {
        Self {
            live_display: LiveDisplay::LinePerTask,
            failure_mode: FailureMode::FailEarly,
            max_parallel_tasks: None,
        }
    }
}
