use crate::run_artifact_writer::RunArtifactWriter;
use crate::run_engine::{TASK_STATUS_PREFIX, TaskExecutionMessage};
use crate::task_output_framer::TaskOutputFramer;
use nao_base::result::NaoResult;
use nao_base::shared_string::SharedString;
use nao_base::timestamp::Timestamp;
use nao_pal::process_event::ProcessEvent;
use nao_pal::process_event_sink::ProcessEventSink;
use nao_pal::process_output_stream::ProcessOutputStream;
use std::sync::mpsc::Sender;

/// Frames one task's process output while also appending task log artifacts as lines arrive.
pub struct LiveTaskArtifactSink {
    writer: RunArtifactWriter,
    task_name: SharedString,
    framer: TaskOutputFramer,
    task_index: usize,
    sender: Sender<TaskExecutionMessage>,
    direct_output: bool,
}

impl LiveTaskArtifactSink {
    /// Creates a sink for one running task.
    pub(crate) fn new(
        writer: RunArtifactWriter,
        task_name: SharedString,
        task_index: usize,
        sender: Sender<TaskExecutionMessage>,
        direct_output: bool,
    ) -> Self {
        let mut framer = TaskOutputFramer::new();
        framer.push_task_heading(task_name.as_str());
        Self {
            writer,
            task_name,
            framer,
            task_index,
            sender,
            direct_output,
        }
    }

    /// Returns the final rendered output and structured log lines.
    pub(crate) fn into_parts(
        self,
    ) -> (
        SharedString,
        Vec<(Timestamp, ProcessOutputStream, String)>,
        Sender<TaskExecutionMessage>,
    ) {
        let (output, log_lines) = self.framer.into_parts();
        (output, log_lines, self.sender)
    }
}

impl ProcessEventSink for LiveTaskArtifactSink {
    fn handle_event(&mut self, event: ProcessEvent) -> NaoResult<()> {
        let previous_line_count = self.framer.log_lines_len();
        self.framer.handle_event(event)?;
        for (timestamp, stream, line) in self.framer.log_lines_since(previous_line_count) {
            self.writer
                .append_task_log_line(&self.task_name, *timestamp, *stream, line)?;
            if self.direct_output {
                self.sender
                    .send(TaskExecutionMessage::OutputLine {
                        task_index: self.task_index,
                        stream: *stream,
                        line: SharedString::from(line.as_str()),
                    })
                    .map_err(|_| nao_base::err!("failed to report task output"))?;
            }
            if let Some(status_message) = line
                .strip_prefix(TASK_STATUS_PREFIX)
                .map(str::trim)
                .filter(|message| !message.is_empty())
            {
                self.writer.append_task_status(
                    self.task_name.as_str(),
                    *timestamp,
                    status_message,
                )?;
                self.sender
                    .send(TaskExecutionMessage::Status {
                        task_index: self.task_index,
                        message: SharedString::from(status_message),
                    })
                    .map_err(|_| nao_base::err!("failed to report task status"))?;
            }
        }
        Ok(())
    }
}
