use nao_base::file_path::FilePath;
use nao_base::shared_string::SharedString;

/// Describes whether a run completed successfully or stopped on a task failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunStatus {
    /// Every planned task completed successfully.
    Completed,
    /// A task exited unsuccessfully.
    Failed(TaskFailure),
}

/// Describes a task failure within an otherwise valid run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskFailure {
    /// Name of the task that failed.
    pub task_name: SharedString,
    /// Process exit code reported for the failed task.
    pub exit_code: i32,
    /// Elapsed run time when the task failed.
    pub elapsed_nanos: u128,
    /// Number of tasks that completed successfully before the failure.
    pub successful_task_count: usize,
}

/// Describes the rendered result of executing a planned run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunExecutionResult {
    /// User-facing rendered output for the executed run.
    pub output: SharedString,
    /// Goal tasks requested by the user.
    pub goal_tasks: Vec<SharedString>,
    /// Total number of tasks in the planned run.
    pub total_task_count: usize,
    /// Total run duration in nanoseconds.
    pub duration_nanos: u128,
    /// Directory that stores the run artifacts.
    pub run_directory: FilePath,
    /// Overall run status.
    pub status: RunStatus,
}
