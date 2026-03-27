use nao_base::err;
use nao_base::result::NaoResult;
use nao_base::shared_string::SharedString;
use nao_recipe::Task;
use std::io::Write as _;
use std::sync::Mutex;
use std::sync::MutexGuard;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LiveTaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LiveTaskState {
    pub(super) name: SharedString,
    pub(super) status: LiveTaskStatus,
    pub(super) elapsed_nanos: Option<u128>,
    pub(super) outcome_message: Option<SharedString>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LiveTaskSnapshot {
    pub(super) header: String,
    pub(super) tasks: Vec<LiveTaskState>,
}

pub fn write_stdout(content: &str) -> NaoResult<()> {
    let mut stdout = std::io::stdout();
    stdout.write_all(content.as_bytes())?;
    stdout.flush()?;
    Ok(())
}

pub(super) fn new_snapshot(header: String, tasks: &[Task]) -> LiveTaskSnapshot {
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

pub(super) fn lock_mutex<'a, T>(
    mutex: &'a Mutex<T>,
    context: &str,
) -> NaoResult<MutexGuard<'a, T>> {
    mutex.lock().map_err(|_| err!("{context}"))
}

pub(super) fn store_async_error(
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

pub(super) fn take_async_error(
    slot: &Mutex<Option<nao_base::error::NaoError>>,
    context: &str,
) -> NaoResult<Option<nao_base::error::NaoError>> {
    let mut slot = lock_mutex(slot, context)?;
    Ok(slot.take())
}

pub(super) fn render_line_per_task_display(
    snapshot: LiveTaskSnapshot,
    running_symbol: &str,
) -> String {
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

pub(super) fn render_single_line_display(snapshot: LiveTaskSnapshot) -> String {
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
