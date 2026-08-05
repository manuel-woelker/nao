use crate::cancellation_token::CancellationToken;
use crate::pal::{FileChangeCallback, Pal, ReadSeek};
use crate::process_command::ProcessCommand;
use crate::process_event::ProcessEvent;
use crate::process_event_sink::ProcessEventSink;
use crate::process_result::ProcessResult;
use expect_test::Expect;
use nao_base::RwLock;
use nao_base::file_path::FilePath;
use nao_base::result::{NaoResult, OptionExt};
use nao_base::timestamp::Timestamp;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Debug;
use std::io::Cursor;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::time::SystemTime;

#[derive(Clone)]
pub struct PalMock {
    inner: Arc<RwLock<PalMockInner>>,
}

struct PalMockInner {
    effects_string: String,
    file_map: HashMap<FilePath, Vec<u8>>,
    file_modified_times: HashMap<FilePath, SystemTime>,
    directories: HashSet<FilePath>,
    process_executions: HashMap<ProcessCommand, (Vec<ProcessEvent>, ProcessResult, Duration)>,
    interactive_terminal: bool,
    default_parallelism: usize,
    current_timestamp: Timestamp,
    current_system_time: SystemTime,
}

impl PalMock {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(PalMockInner {
                effects_string: String::new(),
                file_map: HashMap::new(),
                file_modified_times: HashMap::new(),
                directories: HashSet::new(),
                process_executions: HashMap::new(),
                interactive_terminal: false,
                default_parallelism: 1,
                current_timestamp: Timestamp::new(0),
                current_system_time: SystemTime::UNIX_EPOCH,
            })),
        }
    }

    pub fn log_effect(&self, effect: impl AsRef<str>) {
        let mut inner = self.inner.write();
        inner.effects_string.push_str(effect.as_ref());
        inner.effects_string.push('\n');
    }

    pub fn verify_effects(&self, expected: Expect) {
        expected.assert_eq(&self.inner.read().effects_string);
        self.inner.write().effects_string.clear();
    }

    #[allow(dead_code)]
    pub fn get_effects(&self) -> String {
        self.inner.read().effects_string.clone()
    }

    pub fn clear_effects(&self) {
        self.inner.write().effects_string.clear();
    }

    pub fn set_file(&self, file_path: &str, content: impl Into<Vec<u8>>) {
        let path = FilePath::from(file_path);
        let mut inner = self.inner.write();
        inner.file_map.insert(path.clone(), content.into());
        let current_system_time = inner.current_system_time;
        inner.file_modified_times.insert(path, current_system_time);
    }

    pub fn set_directory(&self, path: &str) {
        self.inner.write().directories.insert(FilePath::from(path));
    }

    pub fn set_process_execution(
        &self,
        command: ProcessCommand,
        events: Vec<ProcessEvent>,
        result: ProcessResult,
    ) {
        self.set_process_execution_with_delay(command, events, result, Duration::ZERO);
    }

    pub fn set_process_execution_with_delay(
        &self,
        command: ProcessCommand,
        events: Vec<ProcessEvent>,
        result: ProcessResult,
        delay: Duration,
    ) {
        self.inner
            .write()
            .process_executions
            .insert(command, (events, result, delay));
    }

    pub fn set_current_timestamp(&self, timestamp: Timestamp) {
        self.inner.write().current_timestamp = timestamp;
    }

    pub fn set_interactive_terminal(&self, interactive_terminal: bool) {
        self.inner.write().interactive_terminal = interactive_terminal;
    }

    pub fn set_default_parallelism(&self, default_parallelism: usize) {
        self.inner.write().default_parallelism = default_parallelism;
    }

    pub fn set_current_system_time(&self, system_time: SystemTime) {
        self.inner.write().current_system_time = system_time;
    }

    pub fn read_file_bytes(&self, path: &str) -> Option<Vec<u8>> {
        self.inner
            .read()
            .file_map
            .get(&FilePath::from(path))
            .cloned()
    }

    pub fn read_file_string(&self, path: &str) -> Option<String> {
        self.read_file_bytes(path)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    }
}

impl Default for PalMock {
    fn default() -> Self {
        Self::new()
    }
}

impl Pal for PalMock {
    fn file_exists(&self, path: &FilePath) -> NaoResult<bool> {
        Ok(self.inner.read().file_map.contains_key(path))
    }

    fn file_modified_time(&self, path: &FilePath) -> NaoResult<SystemTime> {
        self.inner
            .read()
            .file_modified_times
            .get(path)
            .copied()
            .with_context(|| format!("File '{path}' does not exist"))
    }

    fn read_file(&self, path: &FilePath) -> NaoResult<Box<dyn ReadSeek + 'static>> {
        self.log_effect(format!("READ FILE: {path}"));
        Ok(Box::new(Cursor::new(
            self.inner
                .read()
                .file_map
                .get(path)
                .with_context(|| format!("File '{path}' does not exist"))?
                .clone(),
        )))
    }

    fn walk_directory(
        &self,
        path: &FilePath,
        _globs: &[String],
    ) -> NaoResult<Box<dyn Iterator<Item = NaoResult<FilePath>> + '_>> {
        let mut result = vec![];
        for file_path in self.inner.read().file_map.keys() {
            if file_path.as_path().starts_with(path.as_path()) {
                result.push(Ok(file_path.clone()))
            }
        }
        Ok(Box::new(result.into_iter()))
    }

    fn watch_directory(
        &self,
        _directory: &FilePath,
        _globs: &[String],
        _callback: FileChangeCallback,
    ) -> NaoResult<()> {
        Ok(())
    }

    fn create_directory_all(&self, path: &FilePath) -> NaoResult<()> {
        self.log_effect(format!("CREATE DIRECTORY: {path}"));
        self.inner.write().directories.insert(path.clone());
        Ok(())
    }

    fn create_directory(&self, path: &FilePath) -> NaoResult<bool> {
        self.log_effect(format!("CREATE DIRECTORY: {path}"));
        let mut inner = self.inner.write();
        if inner.directories.contains(path) {
            return Ok(false);
        }
        inner.directories.insert(path.clone());
        Ok(true)
    }

    fn write_file(&self, path: &FilePath, content: &[u8]) -> NaoResult<()> {
        self.log_effect(format!(
            "WRITE FILE: {} -> {}",
            path,
            String::from_utf8_lossy(content)
        ));
        let mut inner = self.inner.write();
        inner.file_map.insert(path.clone(), content.to_vec());
        let current_system_time = inner.current_system_time;
        inner
            .file_modified_times
            .insert(path.clone(), current_system_time);
        Ok(())
    }

    fn append_file(&self, path: &FilePath, content: &[u8]) -> NaoResult<()> {
        self.log_effect(format!(
            "APPEND FILE: {} -> {}",
            path,
            String::from_utf8_lossy(content)
        ));
        let mut inner = self.inner.write();
        inner
            .file_map
            .entry(path.clone())
            .and_modify(|existing| existing.extend_from_slice(content))
            .or_insert_with(|| content.to_vec());
        let current_system_time = inner.current_system_time;
        inner
            .file_modified_times
            .insert(path.clone(), current_system_time);
        Ok(())
    }

    fn touch_file(&self, path: &FilePath) -> NaoResult<()> {
        self.log_effect(format!("TOUCH FILE: {path}"));
        let mut inner = self.inner.write();
        inner.file_map.entry(path.clone()).or_default();
        let current_system_time = inner.current_system_time;
        inner
            .file_modified_times
            .insert(path.clone(), current_system_time);
        Ok(())
    }

    fn is_interactive_terminal(&self) -> bool {
        self.inner.read().interactive_terminal
    }

    fn default_parallelism(&self) -> usize {
        self.inner.read().default_parallelism
    }

    fn run_process(
        &self,
        command: &ProcessCommand,
        sink: &mut dyn ProcessEventSink,
    ) -> NaoResult<ProcessResult> {
        self.run_process_cancellable(command, sink, &CancellationToken::new())
    }

    fn run_process_cancellable(
        &self,
        command: &ProcessCommand,
        sink: &mut dyn ProcessEventSink,
        cancellation_token: &CancellationToken,
    ) -> NaoResult<ProcessResult> {
        self.log_effect(format!(
            "RUN PROCESS: {} {}",
            command.executable,
            command
                .arguments
                .iter()
                .map(|argument| argument.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        ));
        let (events, result, delay) = self
            .inner
            .read()
            .process_executions
            .get(command)
            .cloned()
            .with_context(|| {
                format!(
                    "No process execution registered for '{}'",
                    command.executable
                )
            })?;

        if delay > Duration::ZERO {
            thread::sleep(delay);
        }

        for event in events {
            if cancellation_token.is_cancelled() {
                self.log_effect(format!("CANCEL PROCESS: {}", command.executable));
                break;
            }
            sink.handle_event(event)?;
        }

        Ok(result)
    }

    fn now(&self) -> Timestamp {
        self.inner.read().current_timestamp
    }

    fn system_time(&self) -> SystemTime {
        self.inner.read().current_system_time
    }

    fn sleep(&self, duration: Duration) {
        self.log_effect(format!("SLEEP: {}ms", duration.as_millis()));
        self.inner.write().current_system_time += duration;
    }
}

impl Debug for PalMock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PalMock").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::PalMock;
    use crate::cancellation_token::CancellationToken;
    use crate::pal::Pal;
    use crate::process_command::ProcessCommand;
    use crate::process_event::ProcessEvent;
    use crate::process_event_sink::ProcessEventSink;
    use crate::process_exited_event::ProcessExitedEvent;
    use crate::process_output_event::ProcessOutputEvent;
    use crate::process_output_stream::ProcessOutputStream;
    use crate::process_result::ProcessResult;
    use nao_base::result::NaoResult;
    use nao_base::timestamp::Timestamp;

    #[derive(Default)]
    struct RecordingSink {
        events: Vec<ProcessEvent>,
    }

    impl ProcessEventSink for RecordingSink {
        fn handle_event(&mut self, event: ProcessEvent) -> NaoResult<()> {
            self.events.push(event);
            Ok(())
        }
    }

    #[test]
    fn cancellable_process_stops_before_delivering_more_events() {
        let pal = PalMock::new();
        let command = ProcessCommand {
            executable: "server".into(),
            arguments: Vec::new(),
            working_directory: None,
            environment: Vec::new(),
        };
        pal.set_process_execution(
            command.clone(),
            vec![
                ProcessEvent::Output(ProcessOutputEvent {
                    timestamp: Timestamp::new(1),
                    stream: ProcessOutputStream::Stdout,
                    bytes: b"ready\n".to_vec(),
                }),
                ProcessEvent::Exited(ProcessExitedEvent {
                    timestamp: Timestamp::new(2),
                    exit_code: Some(0),
                }),
            ],
            ProcessResult {
                started_at: Timestamp::new(0),
                finished_at: Timestamp::new(2),
                exit_code: Some(0),
            },
        );
        let cancellation_token = CancellationToken::new();
        cancellation_token.cancel();
        let mut sink = RecordingSink::default();

        pal.run_process_cancellable(&command, &mut sink, &cancellation_token)
            .unwrap();

        assert!(sink.events.is_empty());
        assert!(pal.get_effects().contains("CANCEL PROCESS: server"));
    }
}
