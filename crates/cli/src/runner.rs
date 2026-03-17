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

        Ok(self
            .engine
            .execute_run(recipe_path, task_names)?
            .output
            .to_string())
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

        let output = runner
            .execute(&FilePath::from("nao.kdl"), false, &["test".to_owned()])
            .unwrap();

        expect![[r#"
            Running task `build`
            [1ns] stdout: build ok
            [4ns] process exited with code 0

            Running task `test`
            [2ns] stdout: test ok
            [4ns] process exited with code 0
        "#]]
        .assert_eq(&output);
    }
}
