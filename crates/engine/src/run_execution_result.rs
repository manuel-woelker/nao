use nao_base::file_path::FilePath;
use nao_base::shared_string::SharedString;

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
}
