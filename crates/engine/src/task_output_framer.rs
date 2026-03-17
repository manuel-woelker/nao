use nao_base::result::NaoResult;
use nao_base::shared_string::SharedString;
use nao_base::timestamp::Timestamp;
use nao_pal::process_event::ProcessEvent;
use nao_pal::process_event_sink::ProcessEventSink;
use nao_pal::process_exited_event::ProcessExitedEvent;
use nao_pal::process_output_event::ProcessOutputEvent;
use nao_pal::process_output_stream::ProcessOutputStream;
use nao_pal::process_stream_closed_event::ProcessStreamClosedEvent;

/// Buffers raw process chunks and renders timestamp-prefixed task output lines.
pub struct TaskOutputFramer {
    output: String,
    stdout_buffer: Vec<u8>,
    stderr_buffer: Vec<u8>,
}

impl TaskOutputFramer {
    /// Creates a new empty output framer.
    pub fn new() -> Self {
        Self {
            output: String::new(),
            stdout_buffer: Vec::new(),
            stderr_buffer: Vec::new(),
        }
    }

    /// Appends a task heading before process events are received.
    pub fn push_task_heading(&mut self, task_name: &str) {
        if !self.output.is_empty() {
            self.output.push('\n');
        }
        self.output.push_str("Running task `");
        self.output.push_str(task_name);
        self.output.push_str("`\n");
    }

    /// Converts the rendered output into the engine shared string type.
    pub fn into_output(self) -> SharedString {
        SharedString::from(self.output)
    }

    fn handle_output_event(&mut self, event: ProcessOutputEvent) {
        let buffer = match event.stream {
            ProcessOutputStream::Stdout => &mut self.stdout_buffer,
            ProcessOutputStream::Stderr => &mut self.stderr_buffer,
        };
        buffer.extend_from_slice(&event.bytes);
        self.drain_lines(event.stream, event.timestamp);
    }

    fn handle_stream_closed_event(&mut self, event: ProcessStreamClosedEvent) {
        self.flush_partial_line(event.stream, event.timestamp);
    }

    fn handle_exited_event(&mut self, event: ProcessExitedEvent) {
        let exit_code = match event.exit_code {
            Some(exit_code) => exit_code.to_string(),
            None => "unknown".to_owned(),
        };
        self.output.push_str(&format!(
            "[{}] process exited with code {}\n",
            format_timestamp(event.timestamp),
            exit_code
        ));
    }

    fn drain_lines(&mut self, stream: ProcessOutputStream, timestamp: Timestamp) {
        loop {
            let next_line = {
                let buffer = match stream {
                    ProcessOutputStream::Stdout => &mut self.stdout_buffer,
                    ProcessOutputStream::Stderr => &mut self.stderr_buffer,
                };
                let Some(newline_index) = buffer.iter().position(|byte| *byte == b'\n') else {
                    return;
                };
                let mut line = buffer.drain(..=newline_index).collect::<Vec<_>>();
                if line.last() == Some(&b'\n') {
                    line.pop();
                }
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                line
            };
            self.render_line(stream, timestamp, &next_line);
        }
    }

    fn flush_partial_line(&mut self, stream: ProcessOutputStream, timestamp: Timestamp) {
        let remaining = {
            let buffer = match stream {
                ProcessOutputStream::Stdout => &mut self.stdout_buffer,
                ProcessOutputStream::Stderr => &mut self.stderr_buffer,
            };
            if buffer.is_empty() {
                return;
            }
            std::mem::take(buffer)
        };
        self.render_line(stream, timestamp, &remaining);
    }

    fn render_line(&mut self, stream: ProcessOutputStream, timestamp: Timestamp, line: &[u8]) {
        let stream_name = match stream {
            ProcessOutputStream::Stdout => "stdout",
            ProcessOutputStream::Stderr => "stderr",
        };
        self.output.push_str(&format!(
            "[{}] {}: {}\n",
            format_timestamp(timestamp),
            stream_name,
            String::from_utf8_lossy(line)
        ));
    }
}

impl Default for TaskOutputFramer {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessEventSink for TaskOutputFramer {
    fn handle_event(&mut self, event: ProcessEvent) -> NaoResult<()> {
        match event {
            ProcessEvent::Started(_) => {}
            ProcessEvent::Output(event) => self.handle_output_event(event),
            ProcessEvent::StreamClosed(event) => self.handle_stream_closed_event(event),
            ProcessEvent::Exited(event) => self.handle_exited_event(event),
        }
        Ok(())
    }
}

fn format_timestamp(timestamp: Timestamp) -> String {
    format!("{}ns", timestamp.as_nanos())
}

#[cfg(test)]
mod tests {
    use super::TaskOutputFramer;
    use expect_test::expect;
    use nao_base::timestamp::Timestamp;
    use nao_pal::process_event::ProcessEvent;
    use nao_pal::process_event_sink::ProcessEventSink;
    use nao_pal::process_exited_event::ProcessExitedEvent;
    use nao_pal::process_output_event::ProcessOutputEvent;
    use nao_pal::process_output_stream::ProcessOutputStream;
    use nao_pal::process_stream_closed_event::ProcessStreamClosedEvent;

    #[test]
    fn frames_lines_from_raw_chunks() {
        let mut framer = TaskOutputFramer::new();
        framer.push_task_heading("build");
        framer
            .handle_event(ProcessEvent::Output(ProcessOutputEvent {
                timestamp: Timestamp::new(1),
                stream: ProcessOutputStream::Stdout,
                bytes: b"hello ".to_vec(),
            }))
            .unwrap();
        framer
            .handle_event(ProcessEvent::Output(ProcessOutputEvent {
                timestamp: Timestamp::new(2),
                stream: ProcessOutputStream::Stdout,
                bytes: b"world\nagain".to_vec(),
            }))
            .unwrap();
        framer
            .handle_event(ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                timestamp: Timestamp::new(3),
                stream: ProcessOutputStream::Stdout,
            }))
            .unwrap();

        expect![
            r#"Running task `build`
[2ns] stdout: hello world
[3ns] stdout: again
"#
        ]
        .assert_eq(framer.into_output().as_str());
    }

    #[test]
    fn renders_exit_events() {
        let mut framer = TaskOutputFramer::new();
        framer.push_task_heading("test");
        framer
            .handle_event(ProcessEvent::Exited(ProcessExitedEvent {
                timestamp: Timestamp::new(7),
                exit_code: Some(1),
            }))
            .unwrap();

        expect![
            r#"Running task `test`
[7ns] process exited with code 1
"#
        ]
        .assert_eq(framer.into_output().as_str());
    }
}
