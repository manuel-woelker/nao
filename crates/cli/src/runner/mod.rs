mod ci_display;
mod live_display;
mod rendering;

use crate::runner::ci_display::CiDisplay;
use crate::runner::live_display::LinePerTaskDisplay;
use crate::runner::live_display::SingleLineDisplay;
use crate::runner::rendering::render_ci_output;
use crate::runner::rendering::render_failure_summary;
use crate::runner::rendering::render_running_line;
use crate::runner::rendering::render_running_line_body;
use crate::runner::rendering::render_success_summary;
use core::fmt::Write as _;
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_base::result::ResultExt;
use nao_engine::RunEngine;
use nao_engine::RunObserver;
use nao_engine::RunStatus;
use nao_pal::pal::PalHandle;
use nao_recipe::LiveDisplay;
use nao_recipe::Task;
use std::process::ExitCode;

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
            let mut ci_display = CiDisplay::default();
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
            live_display::write_stdout(&render_running_line(&running_line_body))
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
                result.goal_outcome_message.as_deref(),
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

#[cfg(test)]
mod tests {
    use super::Runner;
    use expect_test::expect;
    use nao_base::file_path::FilePath;
    use nao_base::timestamp::Timestamp;
    use nao_pal::pal::PalHandle;
    use nao_pal::pal_mock::PalMock;
    use nao_pal::process_command::ProcessCommand;
    use nao_pal::process_event::ProcessEvent;
    use nao_pal::process_exited_event::ProcessExitedEvent;
    use nao_pal::process_output_event::ProcessOutputEvent;
    use nao_pal::process_output_stream::ProcessOutputStream;
    use nao_pal::process_result::ProcessResult;
    use nao_pal::process_stream_closed_event::ProcessStreamClosedEvent;
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
}
