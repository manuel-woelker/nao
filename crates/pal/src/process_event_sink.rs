use crate::process_event::ProcessEvent;
use nao_base::result::NaoResult;

/// Receives child process lifecycle events during execution.
pub trait ProcessEventSink {
    /// Handles one process event.
    fn handle_event(&mut self, event: ProcessEvent) -> NaoResult<()>;
}
