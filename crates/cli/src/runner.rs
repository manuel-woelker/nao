use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_engine::RunEngine;
use nao_pal::pal::PalHandle;
use nao_recipe::Task;
use std::fmt::Write as _;

/// Executes CLI requests against a recipe file.
pub struct Runner {
    engine: RunEngine,
}

impl Runner {
    /// Creates a new runner for the provided platform abstraction.
    pub fn new(pal: PalHandle) -> Self {
        Self {
            engine: RunEngine::new(pal),
        }
    }

    /// Executes the requested CLI action and returns the rendered output.
    pub fn execute(
        &self,
        recipe_path: &FilePath,
        list: bool,
        task_names: &[String],
    ) -> NaoResult<String> {
        if list {
            return Ok(self.render_task_list(&self.engine.list_tasks(recipe_path)?));
        }

        let result = self.engine.execute_run(recipe_path, task_names)?;
        Ok(render_success_summary(
            &result.goal_tasks,
            result.total_task_count,
            result.duration_nanos,
        ))
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
    goal_tasks: &[nao_base::shared_string::SharedString],
    total_task_count: usize,
    duration_nanos: u128,
) -> String {
    let bold_goal_tasks = format!(
        "\u{1b}[1m{}\u{1b}[0m",
        goal_tasks
            .iter()
            .map(|task| task.as_str())
            .collect::<Vec<_>>()
            .join(",")
    );

    format!(
        "Suceeded {bold_goal_tasks} in {} ({} {})\n",
        pretty_duration(duration_nanos),
        total_task_count,
        if total_task_count == 1 {
            "task"
        } else {
            "tasks"
        }
    )
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
    use super::Runner;
    use super::pretty_duration;
    use super::render_success_summary;
    use expect_test::expect;
    use nao_base::file_path::FilePath;
    use nao_base::shared_string::SharedString;
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
        .assert_eq(&nao_base::unansi(&output));
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
            Suceeded test in 0ns (2 tasks)
        "#]]
        .assert_eq(&nao_base::unansi(&output));
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
            Suceeded lint,test in 2.5ms (5 tasks)
        "#]]
        .assert_eq(&nao_base::unansi(&rendered));
    }
}
