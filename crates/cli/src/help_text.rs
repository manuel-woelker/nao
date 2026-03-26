use crate::version_metadata::VersionMetadata;
use crate::version_metadata::load_version_metadata;
use crate::version_metadata::render_version;

pub(crate) fn render_help(argument_help: &str) -> String {
    format!(
        r#"nao {version}

{argument_help}

Default behavior
  Running `nao` with no task names opens the TUI using `nao.kdl` in the current directory.
  Running `nao build test` executes the requested goal tasks and any dependencies they need.
  Running `nao --list` prints the task names defined in the selected recipe file.
  Running `nao --ci build test` disables interactive progress, prints task lifecycle updates,
  then emits executed task logs and a final run summary.

Task selection
  Task names are passed as positional arguments:
    nao build
    nao build test

  Dependencies run automatically before the requested tasks.

  Task names must not contain `_`.
  `nao` reserves `_` for wildcard selectors, so `test_` matches tasks whose names start with `test`.

Recipe file overview
  `nao` reads a KDL file, usually `nao.kdl`, with one top-level `recipe` node.
  A recipe contains:
    - an optional `config` node
    - one or more `task` nodes

Minimal example
{starter_recipe}

Recipe structure
  recipe "default" {{
    config live-display="line-per-task" max-parallel-tasks=4

    task "build" description="Compile the project" {{
      run shell="cargo build --workspace"
      artifact "workspace-target" path="target"
    }}

    task "test" description="Run tests" {{
      depends-on "build"
      run shell="cargo test --workspace"
      env RUST_LOG="warn"
    }}
  }}

Task nodes
  task "<name>" [description="<text>"] {{
    depends-on "<task-name>"
    run ...
    env NAME="value"
    artifact "<artifact-name>" path="<path>"
  }}

Task properties
  description="<text>"
    Optional human-readable description shown in UI surfaces.

Task child nodes
  depends-on "<task-name>"
    Declare a task dependency. A task may have multiple `depends-on` nodes.

  run shell="<command>"
    Run a shell command.

  run script="<path>"
    Run a script file.

  run container="<image>" {{
    args "--flag" "value"
  }}
    Run a container command.

  env NAME="value"
    Define an environment variable for the task.

  artifact "<name>" path="<path>"
    Declare a produced file or directory as an artifact.

Recipe config
  config live-display="single-line"
  config live-display="line-per-task"
  config max-parallel-tasks=4

  Supported config properties:
    live-display
      Choose how interactive progress is rendered.
      Valid values: `single-line`, `line-per-task`

    max-parallel-tasks
      Limit how many task processes may run at once.
      If omitted, `nao` uses the platform default parallelism.

Task outcomes
  Tasks may report a short human-readable outcome summary by printing a line
  beginning with `Task outcome: `
  Example:
    printf 'Task outcome: 30 tests passed\n'

  Only lines that begin exactly with `Task outcome: ` are captured.
  If multiple outcome lines are produced, the last one wins.
  The outcome line remains in logs and is also persisted for the CLI and TUI.

Authoring rules
  Use exactly one top-level `recipe` node.
  Put tasks inside the recipe node.
  A task must have child nodes and normally includes exactly one `run` node.
  Use `-` instead of `_` in literal task names.
  Keep task names short and easy to type.

Workflow examples
  Initialize a starter file:
    nao --init

  List tasks:
    nao --list

  Run one task:
    nao build

  Run multiple tasks:
    nao fmt clippy test

  Open the TUI with a custom recipe:
    nao --tui --config configs/ci.kdl

"#,
        version = render_version(&load_version_metadata().unwrap_or(VersionMetadata {
            last_commit_date: "unknown".into(),
            short_commit_id: "unknown".into(),
            has_uncommitted_changes: false,
        })),
        argument_help = argument_help.trim_end(),
        starter_recipe = indent_block(crate::recipe_init::starter_recipe(), 2)
    )
}

fn indent_block(value: &str, spaces: usize) -> String {
    let indentation = " ".repeat(spaces);
    value
        .lines()
        .map(|line| format!("{indentation}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::render_help;

    #[test]
    fn help_text_documents_cli_and_recipe_format() {
        let help = render_help(
            r#"ARGS:
    <task_name>...
      Task names or wildcard selectors to execute.

OPTIONS:
    --init
      Create a starter `nao.kdl` in the current directory.

    --list
      List task names from the selected recipe file.

    --tui
      Open the terminal UI.

    --ci
      Run with CI-friendly logging and a final task-log summary.

    --version
      Print build-time version metadata.

    --config <config>
      Load a recipe file other than `nao.kdl`.

    -h, --help
      Prints help information.
"#,
        );

        assert!(help.contains("OPTIONS:"));
        assert!(help.contains("Recipe file overview"));
        assert!(help.contains("Task outcomes"));
        assert!(help.contains("nao --ci build test"));
        assert!(help.contains("run shell=\"<command>\""));
        assert!(help.contains("artifact \"<name>\" path=\"<path>\""));
        assert!(help.contains("nao --init"));
        assert!(help.contains("Only lines that begin exactly with `Task outcome: ` are captured."));
    }
}
