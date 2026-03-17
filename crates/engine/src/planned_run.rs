use nao_recipe::{Task, TaskName};

/// Describes the tasks selected for a run request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedRun {
    /// Top-level tasks requested by the caller.
    pub requested_tasks: Vec<TaskName>,
    /// Concrete tasks selected for the run.
    pub tasks: Vec<Task>,
}
