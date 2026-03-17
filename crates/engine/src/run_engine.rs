use crate::planned_run::PlannedRun;
use crate::run_execution_result::RunExecutionResult;
use crate::task_output_framer::TaskOutputFramer;
use nao_base::err;
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_base::shared_string::SharedString;
use nao_pal::pal::PalHandle;
use nao_pal::process_command::ProcessCommand;
use nao_pal::process_environment_variable::ProcessEnvironmentVariable;
use nao_recipe::{RunSpec, Task, TaskName, load_recipe_with_pal};
use std::collections::{BTreeMap, BTreeSet};

/// Loads recipes, plans runs, and executes tasks.
pub struct RunEngine {
    pal: PalHandle,
}

impl RunEngine {
    /// Creates a new run engine for the provided platform abstraction.
    pub fn new(pal: PalHandle) -> Self {
        Self { pal }
    }

    /// Lists every task declared in the recipe.
    pub fn list_tasks(&self, recipe_path: &FilePath) -> NaoResult<Vec<Task>> {
        Ok(load_recipe_with_pal(&*self.pal, recipe_path)?.tasks)
    }

    /// Plans a run for the requested top-level task names.
    pub fn plan_run(&self, recipe_path: &FilePath, task_names: &[String]) -> NaoResult<PlannedRun> {
        if task_names.is_empty() {
            return Err(err!("usage: nao [--list] [task-name...] [recipe-file]"));
        }

        let recipe = load_recipe_with_pal(&*self.pal, recipe_path)?;
        let mut requested_tasks = Vec::with_capacity(task_names.len());
        let mut tasks = Vec::new();
        let mut visiting = BTreeSet::new();
        let mut visited = BTreeSet::new();
        let task_index = recipe
            .tasks
            .iter()
            .map(|task| (task.name.as_str(), task))
            .collect::<BTreeMap<_, _>>();

        for task_name in task_names {
            requested_tasks.push(TaskName::from(task_name.as_str()));
            self.collect_task(
                &task_index,
                task_name,
                &mut visiting,
                &mut visited,
                &mut tasks,
            )?;
        }

        Ok(PlannedRun {
            requested_tasks,
            tasks,
        })
    }

    /// Executes the planned tasks sequentially and returns rendered output.
    pub fn execute_run(
        &self,
        recipe_path: &FilePath,
        task_names: &[String],
    ) -> NaoResult<RunExecutionResult> {
        let plan = self.plan_run(recipe_path, task_names)?;
        let recipe_directory = recipe_path.parent().unwrap_or_else(|| FilePath::from("."));
        let recipe_directory = if recipe_directory.as_str().is_empty() {
            FilePath::from(".")
        } else {
            recipe_directory
        };
        let mut framer = TaskOutputFramer::new();

        for task in &plan.tasks {
            framer.push_task_heading(task.name.as_str());
            let command = build_process_command(&recipe_directory, task)?;
            let result = self.pal.run_process(&command, &mut framer)?;

            if result.exit_code.unwrap_or(1) != 0 {
                return Err(err!(
                    "task `{}` failed with exit code {}",
                    task.name.as_str(),
                    result.exit_code.unwrap_or(-1)
                ));
            }
        }

        Ok(RunExecutionResult {
            output: framer.into_output(),
        })
    }

    fn collect_task<'task>(
        &self,
        task_index: &BTreeMap<&'task str, &'task Task>,
        task_name: &str,
        visiting: &mut BTreeSet<SharedString>,
        visited: &mut BTreeSet<SharedString>,
        tasks: &mut Vec<Task>,
    ) -> NaoResult<()> {
        if visited.contains(task_name) {
            return Ok(());
        }

        let visiting_name = SharedString::from(task_name);
        if !visiting.insert(visiting_name.clone()) {
            return Err(err!("task dependency cycle detected at `{task_name}`"));
        }

        let task = task_index
            .get(task_name)
            .copied()
            .ok_or_else(|| err!("task `{task_name}` not found"))?;

        for dependency in &task.dependencies {
            self.collect_task(task_index, dependency.as_str(), visiting, visited, tasks)?;
        }

        visiting.remove(&visiting_name);
        visited.insert(visiting_name);
        tasks.push(task.clone());
        Ok(())
    }
}

fn build_process_command(recipe_directory: &FilePath, task: &Task) -> NaoResult<ProcessCommand> {
    let environment = task
        .environment
        .iter()
        .map(|variable| ProcessEnvironmentVariable {
            name: variable.name.clone(),
            value: variable.value.clone(),
        })
        .collect::<Vec<_>>();

    match &task.run {
        RunSpec::Shell(command) => {
            #[cfg(windows)]
            let (executable, arguments) = (
                SharedString::from("cmd"),
                vec![SharedString::from("/C"), command.clone()],
            );

            #[cfg(not(windows))]
            let (executable, arguments) = (
                SharedString::from("sh"),
                vec![SharedString::from("-c"), command.clone()],
            );

            Ok(ProcessCommand {
                executable,
                arguments,
                working_directory: Some(recipe_directory.clone()),
                environment,
            })
        }
        RunSpec::Script(script) => Ok(ProcessCommand {
            executable: SharedString::from(script.as_str()),
            arguments: Vec::new(),
            working_directory: Some(recipe_directory.clone()),
            environment,
        }),
        RunSpec::Container(container) => Err(err!(
            "container execution is not implemented yet for task `{}` with image `{}`",
            task.name.as_str(),
            container.image.as_str()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::RunEngine;
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

    fn test_engine() -> RunEngine {
        let pal = PalMock::new();
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              task "build" description="Build the workspace" {
                run shell="cargo build --workspace --all-targets --all-features"
              }

              task "test" description="Run the test suite" {
                depends-on "build"
                run shell="cargo nextest run --workspace --all-targets --all-features"
              }
            }
            "#,
        );
        RunEngine::new(PalHandle::new(pal))
    }

    fn set_script_process(pal: &PalMock, script: &str, chunks: &[&[u8]]) {
        let mut events = Vec::new();
        for (index, chunk) in chunks.iter().enumerate() {
            events.push(ProcessEvent::Output(ProcessOutputEvent {
                timestamp: Timestamp::new((index + 1) as u128),
                stream: ProcessOutputStream::Stdout,
                bytes: chunk.to_vec(),
            }));
        }
        events.push(ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
            timestamp: Timestamp::new((chunks.len() + 1) as u128),
            stream: ProcessOutputStream::Stdout,
        }));
        events.push(ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
            timestamp: Timestamp::new((chunks.len() + 2) as u128),
            stream: ProcessOutputStream::Stderr,
        }));
        events.push(ProcessEvent::Exited(ProcessExitedEvent {
            timestamp: Timestamp::new((chunks.len() + 3) as u128),
            exit_code: Some(0),
        }));

        pal.set_process_execution(
            ProcessCommand {
                executable: script.into(),
                arguments: Vec::new(),
                working_directory: Some(FilePath::from(".")),
                environment: Vec::new(),
            },
            events,
            ProcessResult {
                started_at: Timestamp::new(0),
                finished_at: Timestamp::new((chunks.len() + 3) as u128),
                exit_code: Some(0),
            },
        );
    }

    #[test]
    fn lists_recipe_tasks() {
        let tasks = test_engine()
            .list_tasks(&FilePath::from("nao.kdl"))
            .unwrap();
        let task_names = tasks
            .iter()
            .map(|task| task.name.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        expect![
            r#"build
test"#
        ]
        .assert_eq(&task_names);
    }

    #[test]
    fn plans_requested_tasks() {
        let plan = test_engine()
            .plan_run(&FilePath::from("nao.kdl"), &["test".to_owned()])
            .unwrap();

        let rendered = format!(
            "requested={}\nplanned={}",
            plan.requested_tasks
                .iter()
                .map(|task| task.as_str())
                .collect::<Vec<_>>()
                .join(","),
            plan.tasks
                .iter()
                .map(|task| task.name.as_str())
                .collect::<Vec<_>>()
                .join(",")
        );

        expect![
            r#"requested=test
planned=build,test"#
        ]
        .assert_eq(&rendered);
    }

    #[test]
    fn executes_tasks_in_dependency_order() {
        let pal = PalMock::new();
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              task "build" {
                run script="./scripts/build.sh"
              }

              task "test" {
                depends-on "build"
                run script="./scripts/test.sh"
              }
            }
            "#,
        );
        set_script_process(&pal, "./scripts/build.sh", &[b"building\n"]);
        set_script_process(&pal, "./scripts/test.sh", &[b"testing"]);
        let engine = RunEngine::new(PalHandle::new(pal.clone()));

        let output = engine
            .execute_run(&FilePath::from("nao.kdl"), &["test".to_owned()])
            .unwrap();

        expect![
            r#"Running task `build`
[1ns] stdout: building
[4ns] process exited with code 0

Running task `test`
[2ns] stdout: testing
[4ns] process exited with code 0
"#
        ]
        .assert_eq(output.output.as_str());
        pal.verify_effects(expect![
            r#"READ FILE: nao.kdl
RUN PROCESS: ./scripts/build.sh 
RUN PROCESS: ./scripts/test.sh 
"#
        ]);
    }
}
