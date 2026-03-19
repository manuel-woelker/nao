use crate::run_artifact_writer::RunArtifactWriter;
use crate::task_output_framer::TaskOutputFramer;
use nao_base::result::NaoResult;
use nao_base::shared_string::SharedString;
use nao_base::timestamp::Timestamp;
use nao_pal::process_event::ProcessEvent;
use nao_pal::process_event_sink::ProcessEventSink;
use nao_pal::process_output_stream::ProcessOutputStream;

/// Frames one task's process output while also appending task log artifacts as lines arrive.
pub struct LiveTaskArtifactSink {
    writer: RunArtifactWriter,
    task_name: SharedString,
    framer: TaskOutputFramer,
}

impl LiveTaskArtifactSink {
    /// Creates a sink for one running task.
    pub fn new(writer: RunArtifactWriter, task_name: SharedString) -> Self {
        let mut framer = TaskOutputFramer::new();
        framer.push_task_heading(task_name.as_str());
        Self {
            writer,
            task_name,
            framer,
        }
    }

    /// Returns the final rendered output and structured log lines.
    pub fn into_parts(self) -> (SharedString, Vec<(Timestamp, ProcessOutputStream, String)>) {
        self.framer.into_parts()
    }
}

impl ProcessEventSink for LiveTaskArtifactSink {
    fn handle_event(&mut self, event: ProcessEvent) -> NaoResult<()> {
        let previous_line_count = self.framer.log_lines_len();
        self.framer.handle_event(event)?;
        for (timestamp, stream, line) in self.framer.log_lines_since(previous_line_count) {
            self.writer
                .append_task_log_line(&self.task_name, *timestamp, *stream, line)?;
        }
        Ok(())
    }
}
