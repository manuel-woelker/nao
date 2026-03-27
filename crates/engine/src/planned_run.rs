use nao_recipe::{FailureMode, LiveDisplay, Task, TaskName};

/// Describes the tasks selected for a run request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedRun {
    /// Top-level tasks requested by the caller.
    pub requested_tasks: Vec<TaskName>,
    /// Interactive live display mode selected by the recipe config.
    pub live_display: LiveDisplay,
    /// Scheduler failure behavior selected by the recipe config.
    pub failure_mode: FailureMode,
    /// Maximum number of task processes that may run at once.
    pub max_parallel_tasks: usize,
    /// Concrete tasks selected for the run.
    pub tasks: Vec<Task>,
}
