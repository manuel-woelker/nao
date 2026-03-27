use nao_base::err;
use nao_base::result::NaoResult;
use nao_base::result::OptionExt;
use nao_base::result::ResultExt;
use nao_base::shared_string::SharedString;
use nao_engine::RunObserver;
use nao_recipe::Task;
use std::io::Write as _;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveTaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveTaskState {
    name: SharedString,
    status: LiveTaskStatus,
    elapsed_nanos: Option<u128>,
    outcome_message: Option<SharedString>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveTaskSnapshot {
    header: String,
    tasks: Vec<LiveTaskState>,
}

pub struct SingleLineDisplay {
    stop: Arc<AtomicBool>,
    snapshot: Arc<Mutex<LiveTaskSnapshot>>,
    update_error: Arc<Mutex<Option<nao_base::error::NaoError>>>,
    handle: Option<thread::JoinHandle<NaoResult<()>>>,
}

impl SingleLineDisplay {
    pub fn start(header: String, tasks: &[Task]) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let snapshot = Arc::new(Mutex::new(new_snapshot(header, tasks)));
        let update_error = Arc::new(Mutex::new(None));
        let thread_stop = Arc::clone(&stop);
        let thread_snapshot = Arc::clone(&snapshot);
        let thread_update_error = Arc::clone(&update_error);
        let handle = thread::spawn(move || {
            const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let mut frame_index = 0usize;

            while !thread_stop.load(Ordering::Relaxed) {
                let rendered = {
                    let snapshot = lock_mutex(
                        &thread_snapshot,
                        "failed to lock single-line display snapshot",
                    )?;
                    render_single_line_display(snapshot.clone())
                };
                write_stdout(&format!("\r{} {}\x1b[K", FRAMES[frame_index], rendered))
                    .context("failed to write single-line display frame")?;
                frame_index = (frame_index + 1) % FRAMES.len();
                thread::sleep(Duration::from_millis(80));
            }

            let rendered = {
                let snapshot = lock_mutex(
                    &thread_snapshot,
                    "failed to lock single-line display snapshot",
                )?;
                render_single_line_display(snapshot.clone())
            };
            let write_result = write_stdout(&format!("\r🚀 {}\x1b[K\n", rendered))
                .context("failed to write final single-line display frame");
            if let Err(error) = write_result {
                store_async_error(
                    &thread_update_error,
                    error,
                    "failed to persist single-line display error",
                );
            }
            Ok(())
        });

        Self {
            stop,
            snapshot,
            update_error,
            handle: Some(handle),
        }
    }

    pub fn finish(&mut self) -> NaoResult<()> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let result = handle
                .join()
                .map_err(|_| err!("single-line display thread panicked"))
                .context("failed to join single-line display thread")?;
            result?;
        }

        if let Some(error) = take_async_error(
            &self.update_error,
            "failed to collect single-line display error",
        )? {
            return Err(error);
        }

        Ok(())
    }

    fn update_task(&self, task_name: &str, status: LiveTaskStatus) {
        self.update_task_with_elapsed(task_name, status, None, None);
    }

    fn update_task_with_elapsed(
        &self,
        task_name: &str,
        status: LiveTaskStatus,
        elapsed_nanos: Option<u128>,
        outcome_message: Option<&str>,
    ) {
        let mut snapshot = match lock_mutex(
            &self.snapshot,
            "failed to lock single-line display snapshot",
        ) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                store_async_error(
                    &self.update_error,
                    error,
                    "failed to persist single-line display update error",
                );
                self.stop.store(true, Ordering::Relaxed);
                return;
            }
        };
        let task = match snapshot
            .tasks
            .iter_mut()
            .find(|task| task.name.as_str() == task_name)
            .with_context(|| format!("task `{task_name}` is missing from single-line display"))
        {
            Ok(task) => task,
            Err(error) => {
                store_async_error(
                    &self.update_error,
                    error,
                    "failed to persist single-line display update error",
                );
                self.stop.store(true, Ordering::Relaxed);
                return;
            }
        };
        task.status = status;
        task.elapsed_nanos = elapsed_nanos;
        task.outcome_message = outcome_message.map(SharedString::from);
    }
}

impl RunObserver for SingleLineDisplay {
    fn on_task_started(&mut self, task_name: &str) {
        self.update_task(task_name, LiveTaskStatus::Running);
    }

    fn on_task_completed(
        &mut self,
        task_name: &str,
        elapsed_nanos: u128,
        outcome_message: Option<&str>,
    ) {
        self.update_task_with_elapsed(
            task_name,
            LiveTaskStatus::Completed,
            Some(elapsed_nanos),
            outcome_message,
        );
    }

    fn on_task_failed(
        &mut self,
        task_name: &str,
        elapsed_nanos: u128,
        outcome_message: Option<&str>,
    ) {
        self.update_task_with_elapsed(
            task_name,
            LiveTaskStatus::Failed,
            Some(elapsed_nanos),
            outcome_message,
        );
    }

    fn on_task_skipped(&mut self, task_name: &str) {
        self.update_task(task_name, LiveTaskStatus::Skipped);
    }
}

impl Drop for SingleLineDisplay {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

pub struct LinePerTaskDisplay {
    stop: Arc<AtomicBool>,
    snapshot: Arc<Mutex<LiveTaskSnapshot>>,
    update_error: Arc<Mutex<Option<nao_base::error::NaoError>>>,
    handle: Option<thread::JoinHandle<NaoResult<()>>>,
}

impl LinePerTaskDisplay {
    pub fn start(header: String, tasks: &[Task]) -> Self {
        let snapshot = Arc::new(Mutex::new(new_snapshot(header, tasks)));
        let stop = Arc::new(AtomicBool::new(false));
        let update_error = Arc::new(Mutex::new(None));
        let thread_stop = Arc::clone(&stop);
        let thread_snapshot = Arc::clone(&snapshot);
        let thread_update_error = Arc::clone(&update_error);
        let handle = thread::spawn(move || {
            const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let line_count = lock_mutex(
                &thread_snapshot,
                "failed to lock line-per-task display snapshot",
            )?
            .tasks
            .len()
                + 1;
            let mut frame_index = 0usize;
            let mut first_render = true;

            loop {
                let rendered = {
                    let snapshot = lock_mutex(
                        &thread_snapshot,
                        "failed to lock line-per-task display snapshot",
                    )?;
                    render_line_per_task_display(snapshot.clone(), FRAMES[frame_index])
                };

                if first_render {
                    write_stdout(&rendered)
                        .context("failed to write initial line-per-task display frame")?;
                    first_render = false;
                } else {
                    write_stdout(&format!("\x1b[{line_count}F\x1b[J{rendered}"))
                        .context("failed to write line-per-task display frame")?;
                }

                if thread_stop.load(Ordering::Relaxed) {
                    break;
                }

                frame_index = (frame_index + 1) % FRAMES.len();
                thread::sleep(Duration::from_millis(80));
            }

            if let Err(error) =
                write_stdout("").context("failed to flush line-per-task display output")
            {
                store_async_error(
                    &thread_update_error,
                    error,
                    "failed to persist line-per-task display error",
                );
            }
            Ok(())
        });

        Self {
            stop,
            snapshot,
            update_error,
            handle: Some(handle),
        }
    }

    pub fn finish(&mut self) -> NaoResult<()> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let result = handle
                .join()
                .map_err(|_| err!("line-per-task display thread panicked"))
                .context("failed to join line-per-task display thread")?;
            result?;
        }

        if let Some(error) = take_async_error(
            &self.update_error,
            "failed to collect line-per-task display error",
        )? {
            return Err(error);
        }

        Ok(())
    }

    fn update_task(&self, task_name: &str, status: LiveTaskStatus) {
        self.update_task_with_elapsed(task_name, status, None, None);
    }

    fn update_task_with_elapsed(
        &self,
        task_name: &str,
        status: LiveTaskStatus,
        elapsed_nanos: Option<u128>,
        outcome_message: Option<&str>,
    ) {
        let mut snapshot = match lock_mutex(
            &self.snapshot,
            "failed to lock line-per-task display snapshot",
        ) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                store_async_error(
                    &self.update_error,
                    error,
                    "failed to persist line-per-task display update error",
                );
                self.stop.store(true, Ordering::Relaxed);
                return;
            }
        };
        let task = match snapshot
            .tasks
            .iter_mut()
            .find(|task| task.name.as_str() == task_name)
            .with_context(|| format!("task `{task_name}` is missing from line-per-task display"))
        {
            Ok(task) => task,
            Err(error) => {
                store_async_error(
                    &self.update_error,
                    error,
                    "failed to persist line-per-task display update error",
                );
                self.stop.store(true, Ordering::Relaxed);
                return;
            }
        };
        task.status = status;
        task.elapsed_nanos = elapsed_nanos;
        task.outcome_message = outcome_message.map(SharedString::from);
    }
}

impl RunObserver for LinePerTaskDisplay {
    fn on_task_started(&mut self, task_name: &str) {
        self.update_task(task_name, LiveTaskStatus::Running);
    }

    fn on_task_completed(
        &mut self,
        task_name: &str,
        elapsed_nanos: u128,
        outcome_message: Option<&str>,
    ) {
        self.update_task_with_elapsed(
            task_name,
            LiveTaskStatus::Completed,
            Some(elapsed_nanos),
            outcome_message,
        );
    }

    fn on_task_failed(
        &mut self,
        task_name: &str,
        elapsed_nanos: u128,
        outcome_message: Option<&str>,
    ) {
        self.update_task_with_elapsed(
            task_name,
            LiveTaskStatus::Failed,
            Some(elapsed_nanos),
            outcome_message,
        );
    }

    fn on_task_skipped(&mut self, task_name: &str) {
        self.update_task(task_name, LiveTaskStatus::Skipped);
    }
}

impl Drop for LinePerTaskDisplay {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

pub fn write_stdout(content: &str) -> NaoResult<()> {
    let mut stdout = std::io::stdout();
    stdout.write_all(content.as_bytes())?;
    stdout.flush()?;
    Ok(())
}

fn new_snapshot(header: String, tasks: &[Task]) -> LiveTaskSnapshot {
    LiveTaskSnapshot {
        header,
        tasks: tasks
            .iter()
            .map(|task| LiveTaskState {
                name: task.name.0.clone(),
                status: LiveTaskStatus::Pending,
                elapsed_nanos: None,
                outcome_message: None,
            })
            .collect(),
    }
}

fn lock_mutex<'a, T>(mutex: &'a Mutex<T>, context: &str) -> NaoResult<MutexGuard<'a, T>> {
    mutex.lock().map_err(|_| err!("{context}"))
}

fn store_async_error(
    slot: &Mutex<Option<nao_base::error::NaoError>>,
    error: nao_base::error::NaoError,
    context: &str,
) {
    match slot.lock() {
        Ok(mut slot) => {
            if slot.is_none() {
                *slot = Some(error);
            }
        }
        Err(_) => eprintln!(
            "{}",
            err!("{context}: {}", error.to_test_string()).to_test_string()
        ),
    }
}

fn take_async_error(
    slot: &Mutex<Option<nao_base::error::NaoError>>,
    context: &str,
) -> NaoResult<Option<nao_base::error::NaoError>> {
    let mut slot = lock_mutex(slot, context)?;
    Ok(slot.take())
}

fn render_line_per_task_display(snapshot: LiveTaskSnapshot, running_symbol: &str) -> String {
    let mut output = String::new();
    let _ = std::fmt::Write::write_fmt(&mut output, format_args!("🚀 {}\n", snapshot.header));
    let task_name_width = snapshot
        .tasks
        .iter()
        .map(|task| task.name.as_str().len())
        .max()
        .unwrap_or(0);
    let duration_width = snapshot
        .tasks
        .iter()
        .filter_map(|task| task.elapsed_nanos)
        .map(format_live_task_runtime_seconds)
        .map(|duration| duration.len())
        .max()
        .unwrap_or(0);
    let outcome_width = snapshot
        .tasks
        .iter()
        .filter_map(|task| task.outcome_message.as_ref())
        .map(|outcome| outcome.as_str().len())
        .max()
        .unwrap_or(0);

    for task in &snapshot.tasks {
        let task_name = format!("{:<task_name_width$}", task.name.as_str());
        let duration = task
            .elapsed_nanos
            .map(format_live_task_runtime_seconds)
            .or_else(|| (task.status == LiveTaskStatus::Running).then(|| "running".to_owned()));
        let outcome = task.outcome_message.as_ref().map(|value| value.as_str());

        match (duration, outcome) {
            (Some(duration), Some(outcome)) => {
                let _ = std::fmt::Write::write_fmt(
                    &mut output,
                    format_args!(
                        "  {} {}  {:>duration_width$}  {:<outcome_width$}\n",
                        render_live_task_status(task.status, running_symbol),
                        task_name,
                        duration,
                        outcome,
                    ),
                );
            }
            (Some(duration), None) => {
                let _ = std::fmt::Write::write_fmt(
                    &mut output,
                    format_args!(
                        "  {} {}  {:>duration_width$}\n",
                        render_live_task_status(task.status, running_symbol),
                        task_name,
                        duration,
                    ),
                );
            }
            (None, Some(outcome)) => {
                let _ = std::fmt::Write::write_fmt(
                    &mut output,
                    format_args!(
                        "  {} {}  {:duration_width$}  {:<outcome_width$}\n",
                        render_live_task_status(task.status, running_symbol),
                        task_name,
                        "",
                        outcome,
                    ),
                );
            }
            (None, None) => {
                let _ = std::fmt::Write::write_fmt(
                    &mut output,
                    format_args!(
                        "  {} {}\n",
                        render_live_task_status(task.status, running_symbol),
                        task_name,
                    ),
                );
            }
        }
    }

    output
}

fn render_single_line_display(snapshot: LiveTaskSnapshot) -> String {
    let running = snapshot
        .tasks
        .iter()
        .filter(|task| task.status == LiveTaskStatus::Running)
        .count();
    let completed = snapshot
        .tasks
        .iter()
        .filter(|task| task.status == LiveTaskStatus::Completed)
        .count();
    let remaining = snapshot
        .tasks
        .iter()
        .filter(|task| matches!(task.status, LiveTaskStatus::Pending))
        .count();

    format!(
        "{} (running: {running}, completed: {completed}, remaining: {remaining})",
        snapshot.header
    )
}

fn render_live_task_status(status: LiveTaskStatus, running_symbol: &str) -> String {
    match status {
        LiveTaskStatus::Pending => "○ ".to_owned(),
        LiveTaskStatus::Running => format!("{running_symbol} "),
        LiveTaskStatus::Completed => "✅".to_owned(),
        LiveTaskStatus::Failed => "\u{1b}[1;31m❌\u{1b}[0m".to_owned(),
        LiveTaskStatus::Skipped => "⏭ ".to_owned(),
    }
}

fn format_live_task_runtime_seconds(duration_nanos: u128) -> String {
    format!("{:.3}s", duration_nanos as f64 / 1_000_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::LiveTaskSnapshot;
    use super::LiveTaskState;
    use super::LiveTaskStatus;
    use super::render_line_per_task_display;
    use super::render_single_line_display;
    use expect_test::expect;
    use nao_base::shared_string::SharedString;

    #[test]
    fn renders_line_per_task_display() {
        let rendered = render_line_per_task_display(
            LiveTaskSnapshot {
                header: "Running test and 1 prerequisite task".to_owned(),
                tasks: vec![
                    LiveTaskState {
                        name: SharedString::from("build"),
                        status: LiveTaskStatus::Completed,
                        elapsed_nanos: Some(4_000_000),
                        outcome_message: Some(SharedString::from("build ready")),
                    },
                    LiveTaskState {
                        name: SharedString::from("test"),
                        status: LiveTaskStatus::Running,
                        elapsed_nanos: None,
                        outcome_message: None,
                    },
                    LiveTaskState {
                        name: SharedString::from("lint"),
                        status: LiveTaskStatus::Running,
                        elapsed_nanos: None,
                        outcome_message: None,
                    },
                    LiveTaskState {
                        name: SharedString::from("publish"),
                        status: LiveTaskStatus::Skipped,
                        elapsed_nanos: None,
                        outcome_message: None,
                    },
                    LiveTaskState {
                        name: SharedString::from("cleanup"),
                        status: LiveTaskStatus::Failed,
                        elapsed_nanos: Some(12_345_678_901),
                        outcome_message: Some(SharedString::from("3 files uploaded")),
                    },
                ],
            },
            "⠙",
        );

        expect![[r#"
            🚀 Running test and 1 prerequisite task
              ✅ build     0.004s  build ready     
              ⠙  test     running
              ⠙  lint     running
              ⏭  publish
              ❌ cleanup  12.346s  3 files uploaded
        "#]]
        .assert_eq(&nao_base::unansi(&rendered));
    }

    #[test]
    fn renders_single_line_display_with_concurrent_progress() {
        let rendered = render_single_line_display(LiveTaskSnapshot {
            header: "Running test and 2 prerequisite tasks".to_owned(),
            tasks: vec![
                LiveTaskState {
                    name: SharedString::from("build"),
                    status: LiveTaskStatus::Completed,
                    elapsed_nanos: Some(4_000_000),
                    outcome_message: None,
                },
                LiveTaskState {
                    name: SharedString::from("lint"),
                    status: LiveTaskStatus::Running,
                    elapsed_nanos: None,
                    outcome_message: None,
                },
                LiveTaskState {
                    name: SharedString::from("test"),
                    status: LiveTaskStatus::Running,
                    elapsed_nanos: None,
                    outcome_message: None,
                },
                LiveTaskState {
                    name: SharedString::from("publish"),
                    status: LiveTaskStatus::Pending,
                    elapsed_nanos: None,
                    outcome_message: None,
                },
            ],
        });

        expect!["Running test and 2 prerequisite tasks (running: 2, completed: 1, remaining: 1)"]
            .assert_eq(&nao_base::unansi(&rendered));
    }
}
