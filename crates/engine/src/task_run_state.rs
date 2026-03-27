/// Tracks the scheduler-visible lifecycle state for one planned task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskRunState {
    /// The task cannot run yet because prerequisites are unfinished.
    Pending,
    /// The task is eligible to start once worker capacity is available.
    Ready,
    /// The task process has been launched and has not finished yet.
    Running,
    /// The task finished successfully.
    Completed,
    /// The task failed.
    Failed,
    /// The task never started because a prior failure made it ineligible to run.
    Skipped,
}
