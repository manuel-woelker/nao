use nao_base::err;
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_pal::pal::PalHandle;
use nao_recipe::{RunSpec, Task, load_recipe_with_pal};
use std::fmt::Write as _;

/// Executes CLI requests against a recipe file.
pub struct Runner {
    pal: PalHandle,
}

impl Runner {
    /// Creates a new runner for the provided platform abstraction.
    pub fn new(pal: PalHandle) -> Self {
        Self { pal }
    }

    /// Executes the requested CLI action and returns the rendered output.
    pub fn execute(
        &self,
        recipe_path: &FilePath,
        list: bool,
        task_names: &[String],
    ) -> NaoResult<String> {
        let recipe = load_recipe_with_pal(&*self.pal, recipe_path)?;

        if list {
            return Ok(self.render_task_list(&recipe.tasks));
        }

        if task_names.is_empty() {
            return Err(err!("usage: nao [--list] [task-name...] [recipe-file]"));
        }

        let mut output = String::new();
        for (index, task_name) in task_names.iter().enumerate() {
            let task = recipe
                .tasks
                .iter()
                .find(|task| task.name.as_str() == task_name)
                .ok_or_else(|| err!("task `{task_name}` not found"))?;

            if index > 0 {
                output.push('\n');
            }
            self.write_task_preview(&mut output, task);
        }

        Ok(output)
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

    fn write_task_preview(&self, output: &mut String, task: &Task) {
        output.push_str("Pretending to run task:\n\n");
        let _ = writeln!(output, "  name: {}", task.name.as_str());

        match &task.description {
            Some(description) => {
                let _ = writeln!(output, "  description: {description}");
            }
            None => output.push_str("  description: <none>\n"),
        }

        if task.dependencies.is_empty() {
            output.push_str("  dependencies: <none>\n");
        } else {
            let dependencies = task
                .dependencies
                .iter()
                .map(|dependency| dependency.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(output, "  dependencies: {dependencies}");
        }

        match &task.run {
            RunSpec::Shell(command) => {
                output.push_str("  run: shell\n");
                let _ = writeln!(output, "  command: {command}");
            }
            RunSpec::Script(script) => {
                output.push_str("  run: script\n");
                let _ = writeln!(output, "  path: {}", script.as_str());
            }
            RunSpec::Container(container) => {
                output.push_str("  run: container\n");
                let _ = writeln!(output, "  image: {}", container.image);
                if container.args.is_empty() {
                    output.push_str("  args: <none>\n");
                } else {
                    let _ = writeln!(output, "  args: {}", container.args.join(" "));
                }
            }
        }

        if task.environment.is_empty() {
            output.push_str("  env: <none>\n");
        } else {
            output.push_str("  env:\n");
            for environment in &task.environment {
                let _ = writeln!(output, "    {}={}", environment.name, environment.value);
            }
        }

        if task.artifacts.is_empty() {
            output.push_str("  artifacts: <none>\n");
        } else {
            output.push_str("  artifacts:\n");
            for artifact in &task.artifacts {
                let _ = writeln!(output, "    {} -> {}", artifact.name, artifact.path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Runner;
    use expect_test::expect;
    use nao_base::file_path::FilePath;
    use nao_pal::pal::PalHandle;
    use nao_pal::pal_mock::PalMock;

    fn test_runner() -> Runner {
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
        Runner::new(PalHandle::new(pal))
    }

    #[test]
    fn renders_task_list() {
        let output = test_runner()
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
    fn renders_selected_task_preview() {
        let output = test_runner()
            .execute(&FilePath::from("nao.kdl"), false, &["test".to_owned()])
            .unwrap();

        expect![[r#"
            Pretending to run task:

              name: test
              description: Run the test suite
              dependencies: build
              run: shell
              command: cargo nextest run --workspace --all-targets --all-features
              env: <none>
              artifacts: <none>
        "#]]
        .assert_eq(&output);
    }
}
