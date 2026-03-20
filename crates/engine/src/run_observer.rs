/// Receives task lifecycle updates while a planned run is executing.
pub trait RunObserver {
    /// Called when a task begins execution.
    fn on_task_started(&mut self, _task_name: &str) {}

    /// Called when a task exits successfully.
    fn on_task_completed(
        &mut self,
        _task_name: &str,
        _elapsed_nanos: u128,
        _outcome_message: Option<&str>,
    ) {
    }

    /// Called when a task fails before the run stops.
    fn on_task_failed(
        &mut self,
        _task_name: &str,
        _elapsed_nanos: u128,
        _outcome_message: Option<&str>,
    ) {
    }

    /// Called when a task is skipped because an earlier task failed.
    fn on_task_skipped(&mut self, _task_name: &str) {}
}
