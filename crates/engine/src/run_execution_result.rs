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
    /// Number of earlier task output lines omitted from the rendered tail.
    pub omitted_output_line_count: usize,
    /// Last task output lines without timestamps.
    pub output_tail_lines: Vec<SharedString>,
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
    /// Final per-task execution results for the run.
    pub task_results: Vec<RunTaskResult>,
    /// Outcome reported by the single requested goal task when available.
    pub goal_outcome_message: Option<SharedString>,
    /// Overall run status.
    pub status: RunStatus,
}

/// Describes the final recorded state of one task in a completed run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunTaskResult {
    /// Task name.
    pub name: SharedString,
    /// Final task status.
    pub status: SharedString,
    /// Final task result string.
    pub result: SharedString,
    /// Process exit code when available.
    pub exit_code: Option<i32>,
    /// Task duration when both start and finish timestamps exist.
    pub duration_nanos: Option<u128>,
    /// Final reported task outcome when available.
    pub outcome_message: Option<SharedString>,
    /// Path to the persisted task log file.
    pub log_path: FilePath,
}
