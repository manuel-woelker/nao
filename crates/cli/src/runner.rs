use core::fmt::Write as _;
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_base::shared_string::SharedString;
use nao_engine::RunEngine;
use nao_engine::RunObserver;
use nao_engine::RunStatus;
use nao_engine::TaskFailure;
use nao_pal::pal::PalHandle;
use nao_recipe::{LiveDisplay, Task, TaskName};
use std::io::Write as _;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

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
        task_names: &[String],
    ) -> NaoResult<RunnerOutput> {
        if list {
            return Ok(RunnerOutput {
                output: self.render_task_list(&self.engine.list_tasks(recipe_path)?),
                exit_code: ExitCode::SUCCESS,
            });
        }

        let plan = self.engine.plan_run(recipe_path, task_names)?;
        let running_line_body = render_running_line_body(&plan.requested_tasks, plan.tasks.len());
        let mut line_per_task_display = None;
        let single_line_display = if self.pal.is_interactive_terminal() {
            match plan.live_display {
                LiveDisplay::SingleLine => Some(RunningSpinner::start(running_line_body.clone())),
                LiveDisplay::LinePerTask => {
                    line_per_task_display = Some(LinePerTaskDisplay::start(
                        running_line_body.clone(),
                        &plan.tasks,
                    ));
                    None
                }
            }
        } else {
            print!("{}", render_running_line(&running_line_body));
            std::io::stdout().flush().unwrap();
            None
        };

        let result = if let Some(display) = line_per_task_display.as_mut() {
            self.engine
                .execute_planned_run_with_observer(recipe_path, &plan, display)?
        } else {
            self.engine.execute_planned_run(recipe_path, &plan)?
        };
        drop(single_line_display);
        drop(line_per_task_display);
        let output = match &result.status {
            RunStatus::Completed => render_success_summary(
                &result.goal_tasks,
                result.total_task_count,
                result.duration_nanos,
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

    fn render_task_list(&self, tasks: &[Task]) -> String {
        let width = tasks
            .iter()
            .map(|task| task.name.as_str().len())
            .max()
            .unwrap_or(0);

        let mut output = String::new();
        output.push_str("Available tasks:\n\n");

        for task in tasks {
            let bold_name = format!("\u{1b}[1m{:<width$}\u{1b}[0m", task.name.as_str());
            match &task.description {
                Some(description) => {
                    let _ = writeln!(&mut output, "  {bold_name}  {description}");
                }
                None => {
                    let _ = writeln!(&mut output, "  {bold_name}");
                }
            }
        }

        output
    }
}

fn render_success_summary(
    goal_tasks: &[SharedString],
    total_task_count: usize,
    duration_nanos: u128,
) -> String {
    let bold_goal_tasks = style_bold_white(&join_goal_tasks(goal_tasks));
    let bold_duration = style_bold_white(&pretty_duration(duration_nanos));

    format!(
        "✅ Suceeded {bold_goal_tasks} in {bold_duration} ({} {})\n",
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

struct RunningSpinner {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl RunningSpinner {
    fn start(line: String) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let mut frame_index = 0usize;

            while !thread_stop.load(Ordering::Relaxed) {
                print!("\r{} {}", FRAMES[frame_index], line);
                std::io::stdout().flush().unwrap();
                frame_index = (frame_index + 1) % FRAMES.len();
                thread::sleep(Duration::from_millis(80));
            }

            print!("\r🚀 {}\x1b[K\n", line);
            std::io::stdout().flush().unwrap();
        });

        Self {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for RunningSpinner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LinePerTaskSnapshot {
    header: String,
    tasks: Vec<LiveTaskState>,
}

struct LinePerTaskDisplay {
    stop: Arc<AtomicBool>,
    snapshot: Arc<Mutex<LinePerTaskSnapshot>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl LinePerTaskDisplay {
    fn start(header: String, tasks: &[Task]) -> Self {
        let snapshot = Arc::new(Mutex::new(LinePerTaskSnapshot {
            header,
            tasks: tasks
                .iter()
                .map(|task| LiveTaskState {
                    name: task.name.0.clone(),
                    status: LiveTaskStatus::Pending,
                })
                .collect(),
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_snapshot = Arc::clone(&snapshot);
        let handle = thread::spawn(move || {
            const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let line_count = thread_snapshot.lock().unwrap().tasks.len() + 1;
            let mut frame_index = 0usize;
            let mut first_render = true;

            loop {
                let rendered = {
                    let snapshot = thread_snapshot.lock().unwrap();
                    render_line_per_task_display(snapshot.clone(), FRAMES[frame_index])
                };

                if first_render {
                    print!("{rendered}");
                    first_render = false;
                } else {
                    print!("\x1b[{line_count}F\x1b[J{rendered}");
                }
                std::io::stdout().flush().unwrap();

                if thread_stop.load(Ordering::Relaxed) {
                    break;
                }

                frame_index = (frame_index + 1) % FRAMES.len();
                thread::sleep(Duration::from_millis(80));
            }
        });

        Self {
            stop,
            snapshot,
            handle: Some(handle),
        }
    }

    fn update_task(&self, task_name: &str, status: LiveTaskStatus) {
        let mut snapshot = self.snapshot.lock().unwrap();
        let task = snapshot
            .tasks
            .iter_mut()
            .find(|task| task.name.as_str() == task_name)
            .unwrap();
        task.status = status;
    }
}

impl RunObserver for LinePerTaskDisplay {
    fn on_task_started(&mut self, task_name: &str) {
        self.update_task(task_name, LiveTaskStatus::Running);
    }

    fn on_task_completed(&mut self, task_name: &str) {
        self.update_task(task_name, LiveTaskStatus::Completed);
    }

    fn on_task_failed(&mut self, task_name: &str) {
        self.update_task(task_name, LiveTaskStatus::Failed);
    }

    fn on_task_skipped(&mut self, task_name: &str) {
        self.update_task(task_name, LiveTaskStatus::Skipped);
    }
}

impl Drop for LinePerTaskDisplay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}

fn render_line_per_task_display(snapshot: LinePerTaskSnapshot, running_symbol: &str) -> String {
    let mut output = String::new();
    let _ = writeln!(&mut output, "🚀 {}", snapshot.header);

    for task in &snapshot.tasks {
        let _ = writeln!(
            &mut output,
            "  {} {}",
            render_live_task_status(task.status, running_symbol),
            task.name.as_str()
        );
    }

    output
}

fn render_live_task_status(status: LiveTaskStatus, running_symbol: &str) -> String {
    match status {
        LiveTaskStatus::Pending => "○".to_owned(),
        LiveTaskStatus::Running => running_symbol.to_owned(),
        LiveTaskStatus::Completed => "✅".to_owned(),
        LiveTaskStatus::Failed => "\u{1b}[1;31m❌\u{1b}[0m".to_owned(),
        LiveTaskStatus::Skipped => "⏭".to_owned(),
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

#[cfg(test)]
mod tests {
    use super::LinePerTaskSnapshot;
    use super::LiveTaskState;
    use super::LiveTaskStatus;
    use super::Runner;
    use super::pretty_duration;
    use super::render_failure_summary;
    use super::render_line_per_task_display;
    use super::render_running_line;
    use super::render_running_line_body;
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
            .execute(&FilePath::from("nao.kdl"), true, &[])
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
    fn executes_selected_tasks() {
        let (runner, pal) = test_runner();
        set_script_process(&pal, "./scripts/build.sh", b"build ok\n");
        set_script_process(&pal, "./scripts/test.sh", b"test ok");
        pal.set_current_timestamp(Timestamp::new(4));

        let output = runner
            .execute(&FilePath::from("nao.kdl"), false, &["test".to_owned()])
            .unwrap();

        expect![[r#"
            ✅ Suceeded test in 0ns (2 tasks)
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
            .execute(&FilePath::from("nao.kdl"), false, &["test".to_owned()])
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
        );

        expect![[r#"
            ✅ Suceeded lint,test in 2.5ms (5 tasks)
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
            LinePerTaskSnapshot {
                header: "Running test and 1 prerequisite task".to_owned(),
                tasks: vec![
                    LiveTaskState {
                        name: SharedString::from("build"),
                        status: LiveTaskStatus::Completed,
                    },
                    LiveTaskState {
                        name: SharedString::from("test"),
                        status: LiveTaskStatus::Running,
                    },
                    LiveTaskState {
                        name: SharedString::from("deploy"),
                        status: LiveTaskStatus::Pending,
                    },
                    LiveTaskState {
                        name: SharedString::from("publish"),
                        status: LiveTaskStatus::Skipped,
                    },
                    LiveTaskState {
                        name: SharedString::from("cleanup"),
                        status: LiveTaskStatus::Failed,
                    },
                ],
            },
            "⠙",
        );

        expect![[r#"
            🚀 Running test and 1 prerequisite task
              ✅ build
              ⠙ test
              ○ deploy
              ⏭ publish
              ❌ cleanup
        "#]]
        .assert_eq(&nao_base::unansi(&rendered));
    }
}
