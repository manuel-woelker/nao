use nao_base::shared_string::SharedString;
use nao_base::timestamp::Timestamp;

/// Captures one observed task lifecycle event for run artifact writing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskEventRecord {
    /// A task was launched by the scheduler.
    Started {
        /// Task name.
        task_name: SharedString,
        /// Observation timestamp.
        timestamp: Timestamp,
    },
    /// A task finished with a final execution result.
    Finished {
        /// Task name.
        task_name: SharedString,
        /// Observation timestamp.
        timestamp: Timestamp,
        /// Final status string.
        status: SharedString,
        /// Final result string.
        result: SharedString,
        /// Exit code when available.
        exit_code: Option<i32>,
    },
    /// A task was skipped after scheduling stopped.
    Skipped {
        /// Task name.
        task_name: SharedString,
        /// Observation timestamp.
        timestamp: Timestamp,
    },
}
