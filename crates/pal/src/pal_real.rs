use crate::cancellation_token::CancellationToken;
use crate::pal::{FileChangeCallback, FileChangeEvent, Pal, PalHandle, ReadSeek};
use crate::process_command::ProcessCommand;
use crate::process_event::ProcessEvent;
use crate::process_event_sink::ProcessEventSink;
use crate::process_exited_event::ProcessExitedEvent;
use crate::process_output_event::ProcessOutputEvent;
use crate::process_output_stream::ProcessOutputStream;
use crate::process_result::ProcessResult;
use crate::process_started_event::ProcessStartedEvent;
use crate::process_stream_closed_event::ProcessStreamClosedEvent;
use ignore::WalkBuilder;
use ignore::gitignore::GitignoreBuilder;
use ignore::overrides::OverrideBuilder;
use nao_base::RwLock;
use nao_base::bail;
use nao_base::file_path::FilePath;
use nao_base::logging::{error, info};
use nao_base::result::{NaoResult, OptionExt, ResultExt};
use nao_base::timestamp::Timestamp;
use notify_debouncer_full::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use std::fmt::Debug;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

const TERMINATION_GRACE_PERIOD: Duration = Duration::from_millis(500);

pub struct PalReal {
    base_path: PathBuf,
    watchers: RwLock<Vec<Debouncer<RecommendedWatcher, RecommendedCache>>>,
    reference_instant: Instant,
    /* 📖 # Why keep the Tokio runtime private to `PalReal`?
    `nao-engine` uses a synchronous PAL boundary so the rest of the workspace does not need to
    depend on Tokio or expose async process types through domain APIs.

    `PalReal` still needs async I/O internally to read child process pipes efficiently on Linux
    and Windows, so it owns one shared runtime and translates those async operations into
    synchronous `run_process` calls with sink-delivered events.
    */
    runtime: Runtime,
}

impl PalReal {
    pub fn new_handle() -> NaoResult<PalHandle> {
        Ok(PalHandle::new(Self::new()?))
    }

    pub fn new() -> NaoResult<Self> {
        let current_dir = std::env::current_dir().context("Unable to access current directory")?;
        let runtime = Runtime::new().context("Unable to create Tokio runtime")?;

        Ok(Self {
            base_path: current_dir,
            watchers: RwLock::new(Vec::new()),
            reference_instant: Instant::now(),
            runtime,
        })
    }

    fn resolve_path(&self, path: &FilePath) -> NaoResult<PathBuf> {
        Ok(self.base_path.join(path.as_path()))
    }

    fn relativize_path(&self, path: &Path) -> NaoResult<FilePath> {
        let relative_path = path.strip_prefix(&self.base_path).with_context(|| {
            format!(
                "Unable to relativize path '{}' against '{}'",
                path.display(),
                self.base_path.display()
            )
        })?;
        Ok(FilePath::new(relative_path))
    }

    fn resolve_process_path(&self, path: &FilePath) -> NaoResult<PathBuf> {
        if path.is_absolute() {
            Ok(path.as_path().to_path_buf())
        } else {
            self.resolve_path(path)
        }
    }

    fn resolve_working_directory(&self, path: &FilePath) -> NaoResult<PathBuf> {
        self.resolve_process_path(path)
    }

    fn timestamp_from(reference_instant: &Instant) -> Timestamp {
        Timestamp::new(reference_instant.elapsed().as_nanos())
    }

    async fn run_process_async(
        &self,
        command: &ProcessCommand,
        sink: &mut dyn ProcessEventSink,
        cancellation_token: &CancellationToken,
    ) -> NaoResult<ProcessResult> {
        let mut child_command = Command::new(command.executable.as_str());
        child_command.args(command.arguments.iter().map(|argument| argument.as_str()));
        child_command.stdout(Stdio::piped());
        child_command.stderr(Stdio::piped());
        configure_child_process(&mut child_command);

        if let Some(working_directory) = &command.working_directory {
            child_command.current_dir(self.resolve_working_directory(working_directory)?);
        }

        for variable in &command.environment {
            child_command.env(variable.name.as_str(), variable.value.as_str());
        }

        let mut child = child_command.spawn().with_context(|| {
            format!("Unable to spawn process '{}'", command.executable.as_str())
        })?;
        let reference_instant = self.reference_instant;
        let started_at = Self::timestamp_from(&reference_instant);
        sink.handle_event(ProcessEvent::Started(ProcessStartedEvent {
            timestamp: started_at,
            process_id: child.id(),
        }))?;

        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut expected_stream_closes = 0usize;

        if let Some(stdout) = child.stdout.take() {
            expected_stream_closes += 1;
            tokio::spawn(read_stream(
                stdout,
                ProcessOutputStream::Stdout,
                reference_instant,
                tx.clone(),
            ));
        }

        if let Some(stderr) = child.stderr.take() {
            expected_stream_closes += 1;
            tokio::spawn(read_stream(
                stderr,
                ProcessOutputStream::Stderr,
                reference_instant,
                tx.clone(),
            ));
        }

        let mut stream_closes = 0usize;
        let mut finished_at = started_at;
        let mut exit_code = None;
        let mut exit_observed = false;
        let mut termination_started_at = None;
        let mut force_kill_requested = false;

        while !exit_observed || stream_closes < expected_stream_closes {
            tokio::select! {
                maybe_event = rx.recv(), if stream_closes < expected_stream_closes => {
                    let event = maybe_event.context("Process output channel closed unexpectedly")?;
                    if let ProcessEvent::StreamClosed(_) = &event {
                        stream_closes += 1;
                    }
                    sink.handle_event(event).with_context(|| {
                        format!(
                            "Unable to deliver process event for '{}'",
                            command.executable.as_str()
                        )
                    })?;
                }
                _ = tokio::time::sleep(Duration::from_millis(20)), if !exit_observed => {
                    if cancellation_token.is_cancelled() && termination_started_at.is_none() {
                        terminate_child(&mut child).with_context(|| {
                            format!(
                                "Unable to terminate process '{}'",
                                command.executable.as_str()
                            )
                        })?;
                        termination_started_at = Some(Instant::now());
                    }

                    if let Some(termination_started_at) = termination_started_at
                        && !force_kill_requested
                        && termination_started_at.elapsed() >= TERMINATION_GRACE_PERIOD
                    {
                        force_kill_child(&mut child).with_context(|| {
                            format!(
                                "Unable to force-kill process '{}'",
                                command.executable.as_str()
                            )
                        })?;
                        force_kill_requested = true;
                    }

                    if let Some(exit_status) = child.try_wait().with_context(|| {
                        format!(
                            "Unable to wait for process '{}'",
                            command.executable.as_str()
                        )
                    })? {
                        finished_at = Self::timestamp_from(&reference_instant);
                        exit_code = exit_status.code();
                        sink.handle_event(ProcessEvent::Exited(ProcessExitedEvent {
                            timestamp: finished_at,
                            exit_code,
                        }))?;
                        exit_observed = true;
                    }
                }
            }
        }

        Ok(ProcessResult {
            started_at,
            finished_at,
            exit_code,
        })
    }
}

#[cfg(unix)]
fn configure_child_process(command: &mut Command) {
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn configure_child_process(_command: &mut Command) {}

#[cfg(unix)]
fn terminate_child(child: &mut Child) -> NaoResult<()> {
    signal_child_process_group(child, libc::SIGTERM)
}

#[cfg(unix)]
fn force_kill_child(child: &mut Child) -> NaoResult<()> {
    signal_child_process_group(child, libc::SIGKILL)
}

#[cfg(unix)]
fn signal_child_process_group(child: &mut Child, signal: libc::c_int) -> NaoResult<()> {
    let process_id = child
        .id()
        .ok_or_else(|| nao_base::err!("child process id is unavailable"))?;
    let process_group_id = -(process_id as libc::pid_t);
    let result = unsafe { libc::kill(process_group_id, signal) };
    if result == -1 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        return Err(error.into());
    }
    Ok(())
}

#[cfg(not(unix))]
fn terminate_child(child: &mut Child) -> NaoResult<()> {
    child.start_kill()?;
    Ok(())
}

#[cfg(not(unix))]
fn force_kill_child(_child: &mut Child) -> NaoResult<()> {
    Ok(())
}

impl Pal for PalReal {
    fn file_exists(&self, path: &FilePath) -> NaoResult<bool> {
        Ok(std::fs::exists(self.resolve_path(path)?)?)
    }

    fn read_file(&self, path: &FilePath) -> NaoResult<Box<dyn ReadSeek + 'static>> {
        Ok(Box::new(
            File::open(self.resolve_path(path)?)
                .with_context(|| format!("Unable to open file '{}'", path))?,
        ))
    }

    fn walk_directory(
        &self,
        path: &FilePath,
        globs: &[String],
    ) -> NaoResult<Box<dyn Iterator<Item = NaoResult<FilePath>> + '_>> {
        let real_path = self.resolve_path(path)?;
        if !real_path.is_dir() {
            bail!("Path is not a directory: '{}'", path);
        }
        let mut walk_builder = WalkBuilder::new(&real_path);
        let mut overrides = OverrideBuilder::new(&real_path);
        for glob in globs {
            overrides.add(glob)?;
        }
        walk_builder.overrides(overrides.build()?);
        Ok(Box::new(
            walk_builder
                .build()
                .filter(|entry| match entry {
                    Ok(dir_entry) => {
                        if let Some(file_type) = &dir_entry.file_type()
                            && file_type.is_file()
                        {
                            true
                        } else {
                            false
                        }
                    }
                    Err(_) => false,
                })
                .flat_map(|entry| entry.map(|path| self.relativize_path(path.path()))),
        ))
    }

    fn watch_directory(
        &self,
        directory: &FilePath,
        globs: &[String],
        callback: FileChangeCallback,
    ) -> NaoResult<()> {
        let mut gitignore_builder = GitignoreBuilder::new(&self.base_path);
        for glob in globs {
            gitignore_builder.add_line(None, glob)?;
        }
        let gitignore = gitignore_builder.build()?;
        let base_path = self.base_path.clone();
        let mut debouncer = new_debouncer(
            Duration::from_millis(500),
            None,
            move |result: DebounceEventResult| match result {
                Ok(events) => {
                    let mut changed_files = Vec::new();
                    for event in &events {
                        if !(event.kind.is_create()
                            || event.kind.is_modify()
                            || event.kind.is_remove())
                        {
                            continue;
                        }
                        for path in &event.paths {
                            let matches = gitignore.matched_path_or_any_parents(path, false);
                            if !matches.is_ignore()
                                && let Ok(relative_path) = path.strip_prefix(&base_path)
                            {
                                changed_files.push(FilePath::new(relative_path));
                            }
                        }
                    }
                    #[allow(clippy::collapsible_if)]
                    if !changed_files.is_empty() {
                        if let Err(error) = callback(FileChangeEvent { changed_files }) {
                            error!("Failed to call filewatcher callback for {events:?}: {error:?}");
                        }
                    }
                }
                Err(errors) => errors.iter().for_each(|error| println!("{error:?}")),
            },
        )?;
        let path = self.resolve_path(directory)?;
        info!(
            "Watching directory {}, globs: {}",
            directory,
            globs.join(", ")
        );
        debouncer.watch(path, RecursiveMode::Recursive)?;
        self.watchers.write().push(debouncer);
        Ok(())
    }

    fn create_directory_all(&self, path: &FilePath) -> NaoResult<()> {
        std::fs::create_dir_all(self.resolve_process_path(path)?)
            .with_context(|| format!("Unable to create directory '{}'", path))?;
        Ok(())
    }

    fn create_directory(&self, path: &FilePath) -> NaoResult<bool> {
        match std::fs::create_dir(self.resolve_process_path(path)?) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(error) => {
                Err(error).with_context(|| format!("Unable to create directory '{}'", path))
            }
        }
    }

    fn write_file(&self, path: &FilePath, content: &[u8]) -> NaoResult<()> {
        std::fs::write(self.resolve_process_path(path)?, content)
            .with_context(|| format!("Unable to write file '{}'", path))?;
        Ok(())
    }

    fn append_file(&self, path: &FilePath, content: &[u8]) -> NaoResult<()> {
        let resolved_path = self.resolve_process_path(path)?;
        if let Some(parent) = resolved_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Unable to create parent directory for '{}'", path))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&resolved_path)
            .with_context(|| format!("Unable to open file '{}' for append", path))?;
        std::io::Write::write_all(&mut file, content)
            .with_context(|| format!("Unable to append file '{}'", path))?;
        Ok(())
    }

    fn is_interactive_terminal(&self) -> bool {
        std::io::stdout().is_terminal()
    }

    fn default_parallelism(&self) -> usize {
        std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
    }

    fn run_process(
        &self,
        command: &ProcessCommand,
        sink: &mut dyn ProcessEventSink,
    ) -> NaoResult<ProcessResult> {
        self.runtime
            .block_on(self.run_process_async(command, sink, &CancellationToken::new()))
    }

    fn run_process_cancellable(
        &self,
        command: &ProcessCommand,
        sink: &mut dyn ProcessEventSink,
        cancellation_token: &CancellationToken,
    ) -> NaoResult<ProcessResult> {
        self.runtime
            .block_on(self.run_process_async(command, sink, cancellation_token))
    }

    fn now(&self) -> Timestamp {
        Timestamp::new(self.reference_instant.elapsed().as_nanos())
    }

    fn system_time(&self) -> SystemTime {
        SystemTime::now()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

impl Debug for PalReal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PalReal").finish()
    }
}

async fn read_stream<R>(
    mut reader: R,
    stream: ProcessOutputStream,
    reference_instant: Instant,
    tx: mpsc::UnboundedSender<ProcessEvent>,
) where
    R: AsyncRead + Unpin,
{
    let mut buffer = [0u8; 4096];

    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => {
                let _ = tx.send(ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                    timestamp: PalReal::timestamp_from(&reference_instant),
                    stream,
                }));
                return;
            }
            Ok(read) => {
                let _ = tx.send(ProcessEvent::Output(ProcessOutputEvent {
                    timestamp: PalReal::timestamp_from(&reference_instant),
                    stream,
                    bytes: buffer[..read].to_vec(),
                }));
            }
            Err(_) => {
                let _ = tx.send(ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                    timestamp: PalReal::timestamp_from(&reference_instant),
                    stream,
                }));
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PalReal;
    use crate::cancellation_token::CancellationToken;
    use crate::pal::Pal;
    use crate::process_command::ProcessCommand;
    use crate::process_event::ProcessEvent;
    use crate::process_event_sink::ProcessEventSink;
    use nao_base::result::NaoResult;
    use nao_base::shared_string::SharedString;
    use std::thread;
    use std::time::Duration;

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
    fn runs_process_and_reports_events() {
        let pal = PalReal::new();
        let mut sink = RecordingSink::default();

        #[cfg(windows)]
        let command = ProcessCommand {
            executable: SharedString::from("cmd"),
            arguments: vec![
                SharedString::from("/C"),
                SharedString::from("(echo hello)&(echo warn 1>&2)"),
            ],
            working_directory: None,
            environment: Vec::new(),
        };

        #[cfg(not(windows))]
        let command = ProcessCommand {
            executable: SharedString::from("sh"),
            arguments: vec![
                SharedString::from("-c"),
                SharedString::from("printf 'hello\\n'; printf 'warn\\n' 1>&2"),
            ],
            working_directory: None,
            environment: Vec::new(),
        };

        let result = pal.unwrap().run_process(&command, &mut sink).unwrap();

        assert_eq!(result.exit_code, Some(0));
        assert!(
            sink.events
                .iter()
                .any(|event| matches!(event, ProcessEvent::Started(_)))
        );
        assert!(sink.events.iter().any(|event| {
            matches!(event, ProcessEvent::Output(output) if String::from_utf8_lossy(&output.bytes).contains("hello"))
        }));
        assert!(sink.events.iter().any(|event| {
            matches!(event, ProcessEvent::Output(output) if String::from_utf8_lossy(&output.bytes).contains("warn"))
        }));
        assert!(
            sink.events
                .iter()
                .any(|event| matches!(event, ProcessEvent::Exited(_)))
        );
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_terminates_subchild_process_group() {
        let pid_file = std::env::temp_dir().join(format!(
            "nao-pal-subchild-{}-{}.pid",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&pid_file);
        let command = ProcessCommand {
            executable: SharedString::from("sh"),
            arguments: vec![
                SharedString::from("-c"),
                SharedString::from(format!(
                    "sleep 30 & printf '%s\\n' \"$!\" > {}; wait",
                    pid_file.display()
                )),
            ],
            working_directory: None,
            environment: Vec::new(),
        };
        let cancellation_token = CancellationToken::new();
        let worker_cancellation_token = cancellation_token.clone();
        let handle = thread::spawn(move || {
            let pal = PalReal::new().unwrap();
            let mut sink = RecordingSink::default();
            pal.run_process_cancellable(&command, &mut sink, &worker_cancellation_token)
                .unwrap()
        });

        for _ in 0..100 {
            if pid_file.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let sleep_pid = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse::<libc::pid_t>()
            .unwrap();

        cancellation_token.cancel();
        let result = handle.join().unwrap();

        assert_ne!(result.exit_code, Some(0));
        for _ in 0..100 {
            if !process_exists(sleep_pid) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!process_exists(sleep_pid));
        let _ = std::fs::remove_file(&pid_file);
    }

    #[cfg(unix)]
    fn process_exists(process_id: libc::pid_t) -> bool {
        let result = unsafe { libc::kill(process_id, 0) };
        result == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
}
