use crate::planned_run::PlannedRun;
use nao_base::err;
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_pal::pal::PalHandle;
use nao_recipe::{Task, TaskName, load_recipe_with_pal};

/// Loads recipes and plans requested runs.
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
        let mut tasks = Vec::with_capacity(task_names.len());

        for task_name in task_names {
            let task = recipe
                .tasks
                .iter()
                .find(|task| task.name.as_str() == task_name)
                .ok_or_else(|| err!("task `{task_name}` not found"))?;

            requested_tasks.push(TaskName::from(task_name.as_str()));
            tasks.push(task.clone());
        }

        Ok(PlannedRun {
            requested_tasks,
            tasks,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::RunEngine;
    use expect_test::expect;
    use nao_base::file_path::FilePath;
    use nao_pal::pal::PalHandle;
    use nao_pal::pal_mock::PalMock;

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
planned=test"#
        ]
        .assert_eq(&rendered);
    }
}
