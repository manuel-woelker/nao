use core::fmt::Write as _;
use nao_base::err;
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_base::result::{OptionExt, ResultExt};
use nao_base::shared_string::SharedString;
use nao_engine::RunEngine;
use nao_engine::RunObserver;
use nao_engine::RunStatus;
use nao_engine::RunTaskResult;
use nao_engine::TaskFailure;
use nao_pal::pal::PalHandle;
use nao_recipe::{LiveDisplay, Task, TaskName};
use std::io::Write as _;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

struct NoopRunObserver;

impl RunObserver for NoopRunObserver {}

/// Describes CLI output and exit status for a runner invocation.
pub struct RunnerOutput {
    /// Rendered user-facing output.
    pub output: String,
    /// Process exit code that should be returned by the CLI.
    pub exit_code: ExitCode,
}

/// Executes CLI requests against a recipe file.
pub struct Runner {
    pal: PalHandle,
    engine: RunEngine,
}

impl Runner {
    /// Creates a new runner for the provided platform abstraction.
    pub fn new(pal: PalHandle) -> Self {
        Self {
            pal: pal.clone(),
            engine: RunEngine::new(pal),
        }
    }

    /// Executes the requested CLI action and returns the rendered output.
    pub fn execute(
        &self,
        recipe_path: &FilePath,
        list: bool,
        ci: bool,
        task_names: &[String],
    ) -> NaoResult<RunnerOutput> {
        if list {
            return Ok(RunnerOutput {
                output: self.render_task_list(
                    &self.engine.list_tasks(recipe_path)?,
                    self.pal.is_interactive_terminal(),
                ),
                exit_code: ExitCode::SUCCESS,
            });
        }

        let run_started_at = self.pal.now();
        let run_started_system_time = self.pal.system_time();
        let plan = self.engine.plan_run(recipe_path, task_names)?;
        if ci {
            let mut ci_display = CiDisplay { write_error: None };
            let result = self.engine.execute_planned_run_with_observer_started_at(
                recipe_path,
                &plan,
                &mut ci_display,
                run_started_at,
                run_started_system_time,
            )?;
            ci_display.finish()?;

            return Ok(RunnerOutput {
                output: render_ci_output(&*self.pal, &result)?,
                exit_code: match result.status {
                    RunStatus::Completed => ExitCode::SUCCESS,
                    RunStatus::Failed(_) => ExitCode::FAILURE,
                },
            });
        }

        let running_line_body = render_running_line_body(&plan.requested_tasks, plan.tasks.len());
        let mut line_per_task_display = None;
        let mut single_line_display = None;
        if self.pal.is_interactive_terminal() {
            match plan.live_display {
                LiveDisplay::SingleLine => {
                    single_line_display = Some(SingleLineDisplay::start(
                        running_line_body.clone(),
                        &plan.tasks,
                    ));
                }
                LiveDisplay::LinePerTask => {
                    line_per_task_display = Some(LinePerTaskDisplay::start(
                        running_line_body.clone(),
                        &plan.tasks,
                    ));
                }
            }
        } else {
            write_stdout(&render_running_line(&running_line_body))
                .context("failed to render run header")?;
        }

        let result = if let Some(display) = line_per_task_display.as_mut() {
            self.engine.execute_planned_run_with_observer_started_at(
                recipe_path,
                &plan,
                display,
                run_started_at,
                run_started_system_time,
            )?
        } else if let Some(display) = single_line_display.as_mut() {
            self.engine.execute_planned_run_with_observer_started_at(
                recipe_path,
                &plan,
                display,
                run_started_at,
                run_started_system_time,
            )?
        } else {
            self.engine.execute_planned_run_with_observer_started_at(
                recipe_path,
                &plan,
                &mut NoopRunObserver,
                run_started_at,
                run_started_system_time,
            )?
        };
        if let Some(display) = single_line_display.as_mut() {
            display.finish()?;
        }
        if let Some(display) = line_per_task_display.as_mut() {
            display.finish()?;
        }
        drop(single_line_display);
        drop(line_per_task_display);
        let output = match &result.status {
            RunStatus::Completed => render_success_summary(
                &result.goal_tasks,
                result.total_task_count,
                result.duration_nanos,
                result
                    .goal_outcome_message
                    .as_ref()
                    .map(|value| value.as_str()),
            ),
            RunStatus::Failed(task_failure) => {
                render_failure_summary(&result.goal_tasks, task_failure)
            }
        };

        Ok(RunnerOutput {
            output,
            exit_code: match result.status {
                RunStatus::Completed => ExitCode::SUCCESS,
                RunStatus::Failed(_) => ExitCode::FAILURE,
            },
        })
    }

    fn render_task_list(&self, tasks: &[Task], interactive_terminal: bool) -> String {
        let width = tasks
            .iter()
            .map(|task| task.name.as_str().len())
            .max()
            .unwrap_or(0);

        let mut output = String::new();
        output.push_str("Available tasks:\n\n");

        for task in tasks {
            let rendered_name = if interactive_terminal {
                format!("\u{1b}[1m{:<width$}\u{1b}[0m", task.name.as_str())
            } else {
                format!("{:<width$}", task.name.as_str())
            };
            match &task.description {
                Some(description) => {
                    let _ = writeln!(&mut output, "  {rendered_name}  {description}");
                }
                None => {
                    let _ = writeln!(&mut output, "  {rendered_name}");
                }
            }
        }

        output
    }
}

struct CiDisplay {
    write_error: Option<nao_base::error::NaoError>,
}

impl CiDisplay {
    fn finish(&mut self) -> NaoResult<()> {
        match self.write_error.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn write_line(&mut self, line: &str, context: &str) {
        if self.write_error.is_some() {
            return;
        }

        if let Err(error) = write_stdout(&(line.to_owned() + "\n")).with_context(|| context) {
            self.write_error = Some(error);
        }
    }
}

impl RunObserver for CiDisplay {
    fn on_task_started(&mut self, task_name: &str) {
        self.write_line(
            &format!("Starting {task_name}"),
            "failed to write CI task start",
        );
    }

    fn on_task_completed(
        &mut self,
        task_name: &str,
        elapsed_nanos: u128,
        outcome_message: Option<&str>,
    ) {
        let outcome_suffix = outcome_message
            .map(|message| format!(": {message}"))
            .unwrap_or_default();
        self.write_line(
            &format!(
                "Completed {task_name} in {}{outcome_suffix}",
                pretty_duration(elapsed_nanos)
            ),
            "failed to write CI task completion",
        );
    }

    fn on_task_failed(
        &mut self,
        task_name: &str,
        elapsed_nanos: u128,
        _outcome_message: Option<&str>,
    ) {
        self.write_line(
            &format!("Failed {task_name} in {}", pretty_duration(elapsed_nanos)),
            "failed to write CI task failure",
        );
    }
}

fn render_success_summary(
    goal_tasks: &[SharedString],
    total_task_count: usize,
    duration_nanos: u128,
    goal_outcome_message: Option<&str>,
) -> String {
    let bold_goal_tasks = style_bold_white(&join_goal_tasks(goal_tasks));
    let bold_duration = style_bold_white(&pretty_duration(duration_nanos));
    let outcome_suffix = goal_outcome_message
        .map(|message| format!(": {}", style_bold_white(message)))
        .unwrap_or_default();

    format!(
        "✅ Succeeded {bold_goal_tasks} in {bold_duration} ({} {}){outcome_suffix}\n",
        total_task_count,
        if total_task_count == 1 {
            "task"
        } else {
            "tasks"
        }
    )
}

fn render_running_line(line_body: &str) -> String {
    format!("🚀 {line_body}\n")
}

fn render_failure_summary(goal_tasks: &[SharedString], task_failure: &TaskFailure) -> String {
    let mut output = String::new();
    let header_text = format!(
        "{} output: ({} lines omitted)",
        task_failure.task_name.as_str(),
        task_failure.omitted_output_line_count,
    );
    let side_width = 9usize;
    let side_line = "─".repeat(side_width);
    let _ = writeln!(
        &mut output,
        "╭{} {} {}╮",
        side_line,
        style_bold_white(&header_text),
        side_line,
    );

    for line in &task_failure.output_tail_lines {
        let _ = writeln!(&mut output, "{}", line.as_str());
    }

    let _ = writeln!(
        &mut output,
        "╰{} {} {}╯",
        side_line,
        style_bold_white(&header_text),
        side_line,
    );
    output.push('\n');

    let _ = writeln!(
        &mut output,
        "\u{1b}[1;31m❌\u{1b}[0m {} failed because {} failed with exit code {} in {} after {} completed successfully",
        style_bold_white(&join_goal_tasks(goal_tasks)),
        style_bold_white(task_failure.task_name.as_str()),
        style_bold_white(&task_failure.exit_code.to_string()),
        style_bold_white(&pretty_duration(task_failure.elapsed_nanos)),
        style_bold_white(&render_completed_task_count(
            task_failure.successful_task_count
        )),
    );

    output
}

fn render_ci_output(
    pal: &dyn nao_pal::pal::Pal,
    result: &nao_engine::RunExecutionResult,
) -> NaoResult<String> {
    let mut output = String::new();
    let mut executed_tasks = result
        .task_results
        .iter()
        .filter(|task| matches!(task.status.as_str(), "completed" | "failed"))
        .collect::<Vec<_>>();
    executed_tasks.sort_by_key(|task| (task.status.as_str() == "failed", task.name.as_str()));

    if !executed_tasks.is_empty() {
        output.push_str("Task logs\n");
        for (index, task) in executed_tasks.iter().enumerate() {
            if index > 0 {
                output.push('\n');
            }
            let _ = writeln!(
                &mut output,
                "== {} ({}) ==",
                task.name.as_str(),
                task.status.as_str()
            );
            output.push_str(&pal.read_file_to_string(&task.log_path)?);
        }
        output.push('\n');
    }

    output.push_str(&render_ci_summary(result));
    Ok(output)
}

fn render_ci_summary(result: &nao_engine::RunExecutionResult) -> String {
    let mut output = String::new();
    let task_name_width = result
        .task_results
        .iter()
        .map(|task| task.name.as_str().len())
        .max()
        .unwrap_or(0);
    let status_width = result
        .task_results
        .iter()
        .map(|task| task.status.as_str().len())
        .max()
        .unwrap_or(0);
    let duration_width = result
        .task_results
        .iter()
        .filter_map(|task| task.duration_nanos)
        .map(pretty_duration)
        .map(|duration| duration.len())
        .max()
        .unwrap_or(1);

    let _ = writeln!(&mut output, "Run summary");
    for task in &result.task_results {
        let duration = task
            .duration_nanos
            .map(pretty_duration)
            .unwrap_or_else(|| "-".to_owned());
        let detail = render_ci_task_summary_detail(task);
        let _ = writeln!(
            &mut output,
            "  {:<status_width$}  {:<task_name_width$}  {:>duration_width$}{}",
            task.status.as_str(),
            task.name.as_str(),
            duration,
            detail
        );
    }

    let _ = writeln!(
        &mut output,
        "\nOverall result: {} in {}",
        match result.status {
            RunStatus::Completed => "completed",
            RunStatus::Failed(_) => "failed",
        },
        pretty_duration(result.duration_nanos)
    );
    if let RunStatus::Failed(task_failure) = &result.status {
        let _ = writeln!(
            &mut output,
            "Failure: {} failed with exit code {}",
            task_failure.task_name.as_str(),
            task_failure.exit_code
        );
    }

    output
}

fn render_ci_task_summary_detail(task: &RunTaskResult) -> String {
    let mut parts = Vec::new();
    if let Some(outcome_message) = &task.outcome_message {
        parts.push(outcome_message.as_str().to_owned());
    }
    if let Some(exit_code) = task.exit_code.filter(|exit_code| *exit_code != 0) {
        parts.push(format!("exit {exit_code}"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("  {}", parts.join(" | "))
    }
}

fn render_running_line_body(goal_tasks: &[TaskName], total_task_count: usize) -> String {
    let prerequisite_task_count = total_task_count.saturating_sub(goal_tasks.len());
    format!(
        "Running {} and {} prerequisite {}",
        style_bold_white(&join_goal_task_names(goal_tasks)),
        prerequisite_task_count,
        if prerequisite_task_count == 1 {
            "task"
        } else {
            "tasks"
        }
    )
}

fn join_goal_tasks(goal_tasks: &[SharedString]) -> String {
    goal_tasks
        .iter()
        .map(|task| task.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

fn join_goal_task_names(goal_tasks: &[TaskName]) -> String {
    goal_tasks
        .iter()
        .map(|task| task.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

fn style_bold_white(text: &str) -> String {
    format!("\u{1b}[1;37m{text}\u{1b}[0m")
}

fn render_completed_task_count(successful_task_count: usize) -> String {
    format!(
        "{successful_task_count} {}",
        if successful_task_count == 1 {
            "task"
        } else {
            "tasks"
        }
    )
}

struct SingleLineDisplay {
    stop: Arc<AtomicBool>,
    snapshot: Arc<Mutex<LiveTaskSnapshot>>,
    update_error: Arc<Mutex<Option<nao_base::error::NaoError>>>,
    handle: Option<thread::JoinHandle<NaoResult<()>>>,
}

impl SingleLineDisplay {
    fn start(header: String, tasks: &[Task]) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let snapshot = Arc::new(Mutex::new(LiveTaskSnapshot {
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
        }));
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

    fn finish(&mut self) -> NaoResult<()> {
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

struct LinePerTaskDisplay {
    stop: Arc<AtomicBool>,
    snapshot: Arc<Mutex<LiveTaskSnapshot>>,
    update_error: Arc<Mutex<Option<nao_base::error::NaoError>>>,
    handle: Option<thread::JoinHandle<NaoResult<()>>>,
}

impl LinePerTaskDisplay {
    fn start(header: String, tasks: &[Task]) -> Self {
        let snapshot = Arc::new(Mutex::new(LiveTaskSnapshot {
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
        }));
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

    fn finish(&mut self) -> NaoResult<()> {
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

fn write_stdout(content: &str) -> NaoResult<()> {
    let mut stdout = std::io::stdout();
    stdout.write_all(content.as_bytes())?;
    stdout.flush()?;
    Ok(())
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
    let _ = writeln!(&mut output, "🚀 {}", snapshot.header);
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
                let _ = writeln!(
                    &mut output,
                    "  {} {}  {:>duration_width$}  {:<outcome_width$}",
                    render_live_task_status(task.status, running_symbol),
                    task_name,
                    duration,
                    outcome,
                );
            }
            (Some(duration), None) => {
                let _ = writeln!(
                    &mut output,
                    "  {} {}  {:>duration_width$}",
                    render_live_task_status(task.status, running_symbol),
                    task_name,
                    duration,
                );
            }
            (None, Some(outcome)) => {
                let _ = writeln!(
                    &mut output,
                    "  {} {}  {:duration_width$}  {:<outcome_width$}",
                    render_live_task_status(task.status, running_symbol),
                    task_name,
                    "",
                    outcome,
                );
            }
            (None, None) => {
                let _ = writeln!(
                    &mut output,
                    "  {} {}",
                    render_live_task_status(task.status, running_symbol),
                    task_name,
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

fn pretty_duration(duration_nanos: u128) -> String {
    if duration_nanos < 1_000 {
        return format!("{duration_nanos}ns");
    }
    if duration_nanos < 1_000_000 {
        return format!("{:.1}us", duration_nanos as f64 / 1_000.0);
    }
    if duration_nanos < 1_000_000_000 {
        return format!("{:.1}ms", duration_nanos as f64 / 1_000_000.0);
    }
    format!("{:.1}s", duration_nanos as f64 / 1_000_000_000.0)
}

fn format_live_task_runtime_seconds(duration_nanos: u128) -> String {
    format!("{:.3}s", duration_nanos as f64 / 1_000_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::LiveTaskSnapshot;
    use super::LiveTaskState;
    use super::LiveTaskStatus;
    use super::Runner;
    use super::pretty_duration;
    use super::render_failure_summary;
    use super::render_line_per_task_display;
    use super::render_running_line;
    use super::render_running_line_body;
    use super::render_single_line_display;
    use super::render_success_summary;
    use expect_test::expect;
    use nao_base::file_path::FilePath;
    use nao_base::shared_string::SharedString;
    use nao_base::timestamp::Timestamp;
    use nao_engine::TaskFailure;
    use nao_pal::pal::PalHandle;
    use nao_pal::pal_mock::PalMock;
    use nao_pal::process_command::ProcessCommand;
    use nao_pal::process_event::ProcessEvent;
    use nao_pal::process_exited_event::ProcessExitedEvent;
    use nao_pal::process_output_event::ProcessOutputEvent;
    use nao_pal::process_output_stream::ProcessOutputStream;
    use nao_pal::process_result::ProcessResult;
    use nao_pal::process_stream_closed_event::ProcessStreamClosedEvent;
    use nao_recipe::TaskName;
    use std::process::ExitCode;

    fn test_runner() -> (Runner, PalMock) {
        let pal = PalMock::new();
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              task "build" description="Build the workspace" {
                run script="./scripts/build.sh"
              }

              task "test" description="Run the test suite" {
                depends-on "build"
                run script="./scripts/test.sh"
              }
            }
            "#,
        );
        (Runner::new(PalHandle::new(pal.clone())), pal)
    }

    fn set_script_process(pal: &PalMock, script: &str, bytes: &[u8]) {
        pal.set_process_execution(
            ProcessCommand {
                executable: script.into(),
                arguments: Vec::new(),
                working_directory: Some(FilePath::from(".")),
                environment: Vec::new(),
            },
            vec![
                ProcessEvent::Output(ProcessOutputEvent {
                    timestamp: Timestamp::new(1),
                    stream: ProcessOutputStream::Stdout,
                    bytes: bytes.to_vec(),
                }),
                ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                    timestamp: Timestamp::new(2),
                    stream: ProcessOutputStream::Stdout,
                }),
                ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                    timestamp: Timestamp::new(3),
                    stream: ProcessOutputStream::Stderr,
                }),
                ProcessEvent::Exited(ProcessExitedEvent {
                    timestamp: Timestamp::new(4),
                    exit_code: Some(0),
                }),
            ],
            ProcessResult {
                started_at: Timestamp::new(0),
                finished_at: Timestamp::new(4),
                exit_code: Some(0),
            },
        );
    }

    #[test]
    fn renders_task_list() {
        let (runner, _) = test_runner();
        let output = runner
            .execute(&FilePath::from("nao.kdl"), true, false, &[])
            .unwrap();

        expect![[r#"
            Available tasks:

              build  Build the workspace
              test   Run the test suite
        "#]]
        .assert_eq(&nao_base::unansi(&output.output));
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
    }

    #[test]
    fn renders_ansi_task_list_for_interactive_terminals() {
        let (runner, pal) = test_runner();
        pal.set_interactive_terminal(true);

        let output = runner
            .execute(&FilePath::from("nao.kdl"), true, false, &[])
            .unwrap();

        assert!(output.output.contains("\u{1b}[1m"));
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
    }

    #[test]
    fn executes_selected_tasks() {
        let (runner, pal) = test_runner();
        set_script_process(&pal, "./scripts/build.sh", b"build ok\n");
        set_script_process(&pal, "./scripts/test.sh", b"test ok");
        pal.set_current_timestamp(Timestamp::new(4));

        let output = runner
            .execute(
                &FilePath::from("nao.kdl"),
                false,
                false,
                &["test".to_owned()],
            )
            .unwrap();

        expect![[r#"
            ✅ Succeeded test in 0ns (2 tasks)
        "#]]
        .assert_eq(&nao_base::unansi(&output.output));
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
    }

    #[test]
    fn renders_failed_task_summary_without_error_wrapper() {
        let (runner, pal) = test_runner();
        set_script_process(&pal, "./scripts/build.sh", b"build ok\n");
        pal.set_process_execution(
            ProcessCommand {
                executable: "./scripts/test.sh".into(),
                arguments: Vec::new(),
                working_directory: Some(FilePath::from(".")),
                environment: Vec::new(),
            },
            vec![
                ProcessEvent::Output(ProcessOutputEvent {
                    timestamp: Timestamp::new(1),
                    stream: ProcessOutputStream::Stdout,
                    bytes: b"boom\n".to_vec(),
                }),
                ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                    timestamp: Timestamp::new(2),
                    stream: ProcessOutputStream::Stdout,
                }),
                ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                    timestamp: Timestamp::new(3),
                    stream: ProcessOutputStream::Stderr,
                }),
                ProcessEvent::Exited(ProcessExitedEvent {
                    timestamp: Timestamp::new(4),
                    exit_code: Some(1),
                }),
            ],
            ProcessResult {
                started_at: Timestamp::new(0),
                finished_at: Timestamp::new(4),
                exit_code: Some(1),
            },
        );

        let output = runner
            .execute(
                &FilePath::from("nao.kdl"),
                false,
                false,
                &["test".to_owned()],
            )
            .unwrap();

        expect![[r#"
            ╭───────── test output: (0 lines omitted) ─────────╮
            boom
            ╰───────── test output: (0 lines omitted) ─────────╯

            ❌ test failed because test failed with exit code 1 in 4ns after 1 task completed successfully
        "#]]
        .assert_eq(&nao_base::unansi(&output.output));
        assert_eq!(output.exit_code, ExitCode::FAILURE);
    }

    #[test]
    fn renders_ci_output_with_task_logs_and_summary() {
        let (runner, pal) = test_runner();
        set_script_process(&pal, "./scripts/build.sh", b"Task outcome: build ready\n");
        set_script_process(
            &pal,
            "./scripts/test.sh",
            b"test ok\nTask outcome: 3 tests passed\n",
        );
        pal.set_current_timestamp(Timestamp::new(9));

        let output = runner
            .execute(
                &FilePath::from("nao.kdl"),
                false,
                true,
                &["test".to_owned()],
            )
            .unwrap();

        expect![[r#"
            Task logs
            == build (completed) ==
            [1970-01-01T00:00:00Z] stdout: Task outcome: build ready

            == test (completed) ==
            [1970-01-01T00:00:00Z] stdout: test ok
            [1970-01-01T00:00:00Z] stdout: Task outcome: 3 tests passed

            Run summary
              completed  build  4ns  build ready
              completed  test   4ns  3 tests passed

            Overall result: completed in 0ns
        "#]]
        .assert_eq(&nao_base::unansi(&output.output));
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
    }

    #[test]
    fn renders_ci_logs_in_alphabetical_order_with_failures_last() {
        let pal = PalMock::new();
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              task "zeta" {
                run script="./scripts/zeta.sh"
              }

              task "alpha" {
                run script="./scripts/alpha.sh"
              }
            }
            "#,
        );
        set_script_process(&pal, "./scripts/zeta.sh", b"zeta ok\n");
        pal.set_process_execution(
            ProcessCommand {
                executable: "./scripts/alpha.sh".into(),
                arguments: Vec::new(),
                working_directory: Some(FilePath::from(".")),
                environment: Vec::new(),
            },
            vec![
                ProcessEvent::Output(ProcessOutputEvent {
                    timestamp: Timestamp::new(1),
                    stream: ProcessOutputStream::Stdout,
                    bytes: b"alpha boom\n".to_vec(),
                }),
                ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                    timestamp: Timestamp::new(2),
                    stream: ProcessOutputStream::Stdout,
                }),
                ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                    timestamp: Timestamp::new(3),
                    stream: ProcessOutputStream::Stderr,
                }),
                ProcessEvent::Exited(ProcessExitedEvent {
                    timestamp: Timestamp::new(4),
                    exit_code: Some(7),
                }),
            ],
            ProcessResult {
                started_at: Timestamp::new(0),
                finished_at: Timestamp::new(4),
                exit_code: Some(7),
            },
        );
        pal.set_current_timestamp(Timestamp::new(9));
        let runner = Runner::new(PalHandle::new(pal));

        let output = runner
            .execute(
                &FilePath::from("nao.kdl"),
                false,
                true,
                &["zeta".to_owned(), "alpha".to_owned()],
            )
            .unwrap();
        let rendered = nao_base::unansi(&output.output);

        let zeta_index = rendered.find("== zeta (completed) ==").unwrap();
        let alpha_index = rendered.find("== alpha (failed) ==").unwrap();
        assert!(zeta_index < alpha_index);
        assert!(rendered.contains("failed     alpha  4ns  exit 7"));
        assert_eq!(output.exit_code, ExitCode::FAILURE);
    }

    #[test]
    fn pretty_prints_durations() {
        expect!["4ns"].assert_eq(&pretty_duration(4));
        expect!["1.5us"].assert_eq(&pretty_duration(1_500));
        expect!["2.5ms"].assert_eq(&pretty_duration(2_500_000));
        expect!["3.0s"].assert_eq(&pretty_duration(3_000_000_000));
    }

    #[test]
    fn renders_multiple_goal_tasks_with_bold_comma_joining() {
        let rendered = render_success_summary(
            &[SharedString::from("lint"), SharedString::from("test")],
            5,
            2_500_000,
            None,
        );

        expect![[r#"
            ✅ Succeeded lint,test in 2.5ms (5 tasks)
        "#]]
        .assert_eq(&nao_base::unansi(&rendered));
    }

    #[test]
    fn renders_running_line_with_prerequisite_count() {
        let rendered = render_running_line(&render_running_line_body(
            &[TaskName::from("lint"), TaskName::from("test")],
            5,
        ));

        expect![[r#"
            🚀 Running lint,test and 3 prerequisite tasks
        "#]]
        .assert_eq(&nao_base::unansi(&rendered));
    }

    #[test]
    fn renders_success_summary_with_goal_outcome() {
        let rendered = render_success_summary(
            &[SharedString::from("test")],
            2,
            2_500_000,
            Some("30 tests succeeded"),
        );

        expect![[r#"
            ✅ Succeeded test in 2.5ms (2 tasks): 30 tests succeeded
        "#]]
        .assert_eq(&nao_base::unansi(&rendered));
    }

    #[test]
    fn renders_failure_summary_for_multiple_completed_tasks() {
        let rendered = render_failure_summary(
            &[SharedString::from("fail5")],
            &TaskFailure {
                task_name: SharedString::from("fail3"),
                exit_code: 1,
                elapsed_nanos: 2_500_000,
                successful_task_count: 2,
                omitted_output_line_count: 0,
                output_tail_lines: vec![
                    SharedString::from("line one"),
                    SharedString::from("line two"),
                ],
            },
        );

        expect![[r#"
            ╭───────── fail3 output: (0 lines omitted) ─────────╮
            line one
            line two
            ╰───────── fail3 output: (0 lines omitted) ─────────╯

            ❌ fail5 failed because fail3 failed with exit code 1 in 2.5ms after 2 tasks completed successfully
        "#]]
        .assert_eq(&nao_base::unansi(&rendered));
    }

    #[test]
    fn renders_failure_summary_with_omitted_line_notice() {
        let rendered = render_failure_summary(
            &[SharedString::from("fail5")],
            &TaskFailure {
                task_name: SharedString::from("fail3"),
                exit_code: 1,
                elapsed_nanos: 2_500_000,
                successful_task_count: 2,
                omitted_output_line_count: 23,
                output_tail_lines: vec![
                    SharedString::from("kept line 1"),
                    SharedString::from("kept line 2"),
                ],
            },
        );

        expect![[r#"
            ╭───────── fail3 output: (23 lines omitted) ─────────╮
            kept line 1
            kept line 2
            ╰───────── fail3 output: (23 lines omitted) ─────────╯

            ❌ fail5 failed because fail3 failed with exit code 1 in 2.5ms after 2 tasks completed successfully
        "#]]
        .assert_eq(&nao_base::unansi(&rendered));
    }

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
