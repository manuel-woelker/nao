mod runner;

use nao_base::err;
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_base::shared_string::SharedString;
use nao_pal::pal::Pal;
use nao_pal::pal::PalHandle;
use nao_pal::pal_real::PalReal;
use runner::Runner;
use std::path::PathBuf;
use std::process::ExitCode;

shadow_rs::shadow!(build);

xflags::xflags! {
    /// Run local task graphs defined in a `nao.kdl` recipe.
    cmd nao {
        /// Create a starter `nao.kdl` in the current directory.
        optional --init
        /// List task names from the selected recipe file.
        optional --list
        /// Open the terminal UI.
        optional --tui
        /// Print build-time version metadata.
        optional --version
        /// Load a recipe file other than `nao.kdl`.
        optional --config config: PathBuf
        /// Task names or wildcard selectors to execute.
        repeated task_name: String
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprint!(
                "{}",
                nao_base::cli::format_cli_error("nao CLI failed", &error)
            );
            ExitCode::FAILURE
        }
    }
}

fn run() -> NaoResult<ExitCode> {
    let raw_args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let flags = match Nao::from_vec(raw_args) {
        Ok(flags) => flags,
        Err(error) if error.is_help() => {
            print!("{}", render_help(&error.to_string()));
            return Ok(ExitCode::SUCCESS);
        }
        Err(error) => return Err(err!("{error}")),
    };
    let pal = PalReal::new_handle();

    run_with_pal_and_version_loader(flags, pal, load_version_metadata)
}

fn run_with_pal_and_version_loader<F>(
    flags: Nao,
    pal: PalHandle,
    load_version_metadata: F,
) -> NaoResult<ExitCode>
where
    F: FnOnce() -> NaoResult<VersionMetadata>,
{
    if flags.version {
        validate_version_request(&flags)?;
        println!("{}", render_version(&load_version_metadata()?));
        return Ok(ExitCode::SUCCESS);
    }

    if flags.init {
        validate_init_request(&flags)?;
        initialize_recipe_file(&*pal, &FilePath::from("nao.kdl"))?;
        return Ok(ExitCode::SUCCESS);
    }

    if should_run_tui(&flags) {
        validate_tui_request(&flags)?;
        let recipe_path = flags
            .config
            .clone()
            .unwrap_or_else(|| PathBuf::from("nao.kdl"));
        nao_tui::run(pal.clone(), FilePath::new(&recipe_path))?;
        return Ok(ExitCode::SUCCESS);
    }

    let recipe_path = flags.config.unwrap_or_else(|| PathBuf::from("nao.kdl"));
    let runner = Runner::new(pal);
    let output = runner.execute(&FilePath::new(&recipe_path), flags.list, &flags.task_name)?;

    print!("{}", output.output);
    Ok(output.exit_code)
}

fn should_run_tui(flags: &Nao) -> bool {
    flags.tui || (!flags.version && !flags.init && !flags.list && flags.task_name.is_empty())
}

fn validate_tui_request(flags: &Nao) -> NaoResult<()> {
    if flags.list {
        return Err(err!("--tui cannot be combined with --list"));
    }
    if !flags.task_name.is_empty() {
        return Err(err!("--tui cannot be combined with task names"));
    }
    Ok(())
}

fn validate_version_request(flags: &Nao) -> NaoResult<()> {
    if flags.init {
        return Err(err!("--version cannot be combined with --init"));
    }
    if flags.list {
        return Err(err!("--version cannot be combined with --list"));
    }
    if flags.tui {
        return Err(err!("--version cannot be combined with --tui"));
    }
    if flags.config.is_some() {
        return Err(err!("--version cannot be combined with --config"));
    }
    if !flags.task_name.is_empty() {
        return Err(err!("--version cannot be combined with task names"));
    }
    Ok(())
}

fn validate_init_request(flags: &Nao) -> NaoResult<()> {
    if flags.list {
        return Err(err!("--init cannot be combined with --list"));
    }
    if flags.tui {
        return Err(err!("--init cannot be combined with --tui"));
    }
    if flags.config.is_some() {
        return Err(err!("--init cannot be combined with --config"));
    }
    if !flags.task_name.is_empty() {
        return Err(err!("--init cannot be combined with task names"));
    }
    Ok(())
}

fn initialize_recipe_file(pal: &dyn Pal, path: &FilePath) -> NaoResult<()> {
    if pal.file_exists(path)? {
        return Err(err!("{path} already exists"));
    }

    pal.write_file(path, starter_recipe().as_bytes())?;
    println!("Created {path}");
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VersionMetadata {
    last_commit_date: SharedString,
    short_commit_id: SharedString,
    has_uncommitted_changes: bool,
}

fn render_version(metadata: &VersionMetadata) -> String {
    let dev_suffix = if metadata.has_uncommitted_changes {
        "-dev"
    } else {
        ""
    };

    format!(
        "{}-{}-{}{}",
        env!("CARGO_PKG_VERSION"),
        metadata.last_commit_date.as_str(),
        metadata.short_commit_id.as_str(),
        dev_suffix
    )
}

fn load_version_metadata() -> NaoResult<VersionMetadata> {
    Ok(VersionMetadata {
        last_commit_date: SharedString::from(normalize_commit_date(build::COMMIT_DATE)),
        short_commit_id: SharedString::from(normalize_short_commit(build::SHORT_COMMIT)),
        has_uncommitted_changes: !build::GIT_CLEAN,
    })
}

fn normalize_commit_date(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 10 {
        trimmed[..10].to_owned()
    } else if trimmed.is_empty() {
        "unknown".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn normalize_short_commit(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "unknown".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn starter_recipe() -> &'static str {
    r#"recipe "default" {
  task "build" description="Sample build task using direct outcome output" {
    run shell="""
      printf 'Building the project...\n'
      printf 'Task outcome: build artifacts are ready\n'
    """
  }

  task "test" description="Sample test task using the NAO_TASK_OUTCOME helper" {
    depends-on "build"
    run shell="""
      printf 'Running sample tests...\n'
      NAO_TASK_OUTCOME="3 sample tests passed"
    """
  }
}
"#
}

fn render_help(argument_help: &str) -> String {
    format!(
        r#"nao {version}

{argument_help}

Default behavior
  Running `nao` with no task names opens the TUI using `nao.kdl` in the current directory.
  Running `nao build test` executes the requested goal tasks and any dependencies they need.
  Running `nao --list` prints the task names defined in the selected recipe file.

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
  Tasks may report a short human-readable outcome summary in either of these ways.

  1. Print a line beginning with `Task outcome: `
     Example:
       printf 'Task outcome: 30 tests passed\n'

  2. For Unix `run shell` tasks, set `NAO_TASK_OUTCOME`
     Example:
       NAO_TASK_OUTCOME="30 tests passed"

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

For more detail, inspect `docs/RECIPES.md`, but `--help` should be enough to get moving without that file.
"#,
        version = render_version(&load_version_metadata().unwrap_or(VersionMetadata {
            last_commit_date: SharedString::from("unknown"),
            short_commit_id: SharedString::from("unknown"),
            has_uncommitted_changes: false,
        })),
        argument_help = argument_help.trim_end(),
        starter_recipe = indent_block(starter_recipe(), 2)
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
    use super::Nao;
    use super::VersionMetadata;
    use super::initialize_recipe_file;
    use super::normalize_commit_date;
    use super::normalize_short_commit;
    use super::render_help;
    use super::render_version;
    use super::run_with_pal_and_version_loader;
    use super::should_run_tui;
    use super::starter_recipe;
    use super::validate_init_request;
    use super::validate_tui_request;
    use super::validate_version_request;
    use expect_test::expect;
    use nao_base::file_path::FilePath;
    use nao_base::shared_string::SharedString;
    use nao_pal::pal::PalHandle;
    use nao_pal::pal_mock::PalMock;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::process::ExitCode;

    #[test]
    fn parses_config_flag_and_tasks() {
        let flags = Nao::from_vec(vec![
            OsString::from("--config"),
            OsString::from("configs/custom.kdl"),
            OsString::from("build"),
            OsString::from("test"),
        ])
        .unwrap();

        assert_eq!(flags.config, Some(PathBuf::from("configs/custom.kdl")));
        assert_eq!(flags.task_name, vec!["build".to_owned(), "test".to_owned()]);
    }

    #[test]
    fn defaults_to_no_config_flag() {
        let flags = Nao::from_vec(vec![OsString::from("--list")]).unwrap();

        assert_eq!(flags.config, None);
        assert!(flags.list);
    }

    #[test]
    fn parses_tui_flag() {
        let flags = Nao::from_vec(vec![OsString::from("--tui")]).unwrap();

        assert!(flags.tui);
        assert_eq!(flags.config, None);
        assert!(flags.task_name.is_empty());
    }

    #[test]
    fn parses_init_flag() {
        let flags = Nao::from_vec(vec![OsString::from("--init")]).unwrap();

        assert!(flags.init);
        assert!(!flags.version);
        assert!(!flags.list);
        assert!(!flags.tui);
        assert_eq!(flags.config, None);
        assert!(flags.task_name.is_empty());
    }

    #[test]
    fn parses_version_flag() {
        let flags = Nao::from_vec(vec![OsString::from("--version")]).unwrap();

        assert!(flags.version);
        assert!(!flags.init);
        assert!(!flags.list);
        assert!(!flags.tui);
        assert_eq!(flags.config, None);
        assert!(flags.task_name.is_empty());
    }

    #[test]
    fn rejects_list_with_tui() {
        let flags = Nao::from_vec(vec![OsString::from("--tui"), OsString::from("--list")]).unwrap();

        let error = validate_tui_request(&flags).unwrap_err();

        assert!(
            error
                .to_test_string()
                .contains("--tui cannot be combined with --list")
        );
    }

    #[test]
    fn rejects_task_names_with_tui() {
        let flags = Nao::from_vec(vec![OsString::from("--tui"), OsString::from("build")]).unwrap();

        let error = validate_tui_request(&flags).unwrap_err();

        assert!(
            error
                .to_test_string()
                .contains("--tui cannot be combined with task names")
        );
    }

    #[test]
    fn rejects_init_with_version() {
        let flags =
            Nao::from_vec(vec![OsString::from("--version"), OsString::from("--init")]).unwrap();

        let error = validate_version_request(&flags).unwrap_err();

        assert!(
            error
                .to_test_string()
                .contains("--version cannot be combined with --init")
        );
    }

    #[test]
    fn rejects_config_with_version() {
        let flags = Nao::from_vec(vec![
            OsString::from("--version"),
            OsString::from("--config"),
            OsString::from("custom.kdl"),
        ])
        .unwrap();

        let error = validate_version_request(&flags).unwrap_err();

        assert!(
            error
                .to_test_string()
                .contains("--version cannot be combined with --config")
        );
    }

    #[test]
    fn rejects_task_names_with_version() {
        let flags =
            Nao::from_vec(vec![OsString::from("--version"), OsString::from("build")]).unwrap();

        let error = validate_version_request(&flags).unwrap_err();

        assert!(
            error
                .to_test_string()
                .contains("--version cannot be combined with task names")
        );
    }

    #[test]
    fn rejects_list_with_init() {
        let flags =
            Nao::from_vec(vec![OsString::from("--init"), OsString::from("--list")]).unwrap();

        let error = validate_init_request(&flags).unwrap_err();

        assert!(
            error
                .to_test_string()
                .contains("--init cannot be combined with --list")
        );
    }

    #[test]
    fn rejects_config_with_init() {
        let flags = Nao::from_vec(vec![
            OsString::from("--init"),
            OsString::from("--config"),
            OsString::from("custom.kdl"),
        ])
        .unwrap();

        let error = validate_init_request(&flags).unwrap_err();

        assert!(
            error
                .to_test_string()
                .contains("--init cannot be combined with --config")
        );
    }

    #[test]
    fn rejects_task_names_with_init() {
        let flags = Nao::from_vec(vec![OsString::from("--init"), OsString::from("build")]).unwrap();

        let error = validate_init_request(&flags).unwrap_err();

        assert!(
            error
                .to_test_string()
                .contains("--init cannot be combined with task names")
        );
    }

    #[test]
    fn defaults_to_tui_when_no_action_is_given() {
        let flags = Nao::from_vec(Vec::<OsString>::new()).unwrap();

        assert!(should_run_tui(&flags));
    }

    #[test]
    fn does_not_default_to_tui_when_listing_tasks() {
        let flags = Nao::from_vec(vec![OsString::from("--list")]).unwrap();

        assert!(!should_run_tui(&flags));
    }

    #[test]
    fn does_not_default_to_tui_when_tasks_are_requested() {
        let flags = Nao::from_vec(vec![OsString::from("build")]).unwrap();

        assert!(!should_run_tui(&flags));
    }

    #[test]
    fn does_not_default_to_tui_when_init_is_requested() {
        let flags = Nao::from_vec(vec![OsString::from("--init")]).unwrap();

        assert!(!should_run_tui(&flags));
    }

    #[test]
    fn does_not_default_to_tui_when_version_is_requested() {
        let flags = Nao::from_vec(vec![OsString::from("--version")]).unwrap();

        assert!(!should_run_tui(&flags));
    }

    #[test]
    fn renders_version_without_dev_suffix_for_clean_worktree() {
        let rendered = render_version(&VersionMetadata {
            last_commit_date: SharedString::from("2026-03-21"),
            short_commit_id: SharedString::from("abc1234"),
            has_uncommitted_changes: false,
        });

        assert_eq!(
            rendered,
            format!("{}-2026-03-21-abc1234", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn renders_version_with_dev_suffix_for_dirty_worktree() {
        let rendered = render_version(&VersionMetadata {
            last_commit_date: SharedString::from("2026-03-21"),
            short_commit_id: SharedString::from("abc1234"),
            has_uncommitted_changes: true,
        });

        assert_eq!(
            rendered,
            format!("{}-2026-03-21-abc1234-dev", env!("CARGO_PKG_VERSION"))
        );
    }

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
        assert!(help.contains("run shell=\"<command>\""));
        assert!(help.contains("artifact \"<name>\" path=\"<path>\""));
        assert!(help.contains("nao --init"));
    }

    #[test]
    fn normalizes_shadow_commit_date_to_calendar_date() {
        assert_eq!(
            normalize_commit_date("2026-03-21 14:22:11 +00:00"),
            "2026-03-21"
        );
    }

    #[test]
    fn falls_back_to_unknown_when_shadow_commit_date_is_missing() {
        assert_eq!(normalize_commit_date(""), "unknown");
    }

    #[test]
    fn falls_back_to_unknown_when_shadow_short_commit_is_missing() {
        assert_eq!(normalize_short_commit(""), "unknown");
    }

    #[test]
    fn init_writes_starter_recipe_when_missing() {
        let pal = PalMock::new();

        initialize_recipe_file(&pal, &FilePath::from("nao.kdl")).unwrap();

        expect![[r#"
            WRITE FILE: nao.kdl -> recipe "default" {
              task "build" description="Sample build task using direct outcome output" {
                run shell="""
                  printf 'Building the project...\n'
                  printf 'Task outcome: build artifacts are ready\n'
                """
              }

              task "test" description="Sample test task using the NAO_TASK_OUTCOME helper" {
                depends-on "build"
                run shell="""
                  printf 'Running sample tests...\n'
                  NAO_TASK_OUTCOME="3 sample tests passed"
                """
              }
            }

        "#]]
        .assert_eq(&pal.get_effects());
        assert_eq!(
            pal.read_file_string("nao.kdl").as_deref(),
            Some(starter_recipe())
        );
    }

    #[test]
    fn init_keeps_existing_recipe_file() {
        let pal = PalMock::new();
        pal.set_file("nao.kdl", "recipe \"existing\" {}");

        let error = initialize_recipe_file(&pal, &FilePath::from("nao.kdl")).unwrap_err();

        assert!(error.to_test_string().contains("nao.kdl already exists"));
        assert_eq!(pal.get_effects(), "");
        assert_eq!(
            pal.read_file_string("nao.kdl").as_deref(),
            Some("recipe \"existing\" {}")
        );
    }

    #[test]
    fn run_with_init_returns_success() {
        let pal = PalMock::new();
        let flags = Nao::from_vec(vec![OsString::from("--init")]).unwrap();

        let exit_code = run_with_pal_and_version_loader(flags, PalHandle::new(pal.clone()), || {
            unreachable!("--init should not load version metadata")
        })
        .unwrap();

        assert_eq!(exit_code, ExitCode::SUCCESS);
        assert_eq!(
            pal.read_file_string("nao.kdl").as_deref(),
            Some(starter_recipe())
        );
    }

    #[test]
    fn run_with_init_returns_error_when_recipe_exists() {
        let pal = PalMock::new();
        pal.set_file("nao.kdl", "recipe \"existing\" {}");
        let flags = Nao::from_vec(vec![OsString::from("--init")]).unwrap();

        let error = run_with_pal_and_version_loader(flags, PalHandle::new(pal.clone()), || {
            unreachable!("--init should not load version metadata")
        })
        .unwrap_err();

        assert!(error.to_test_string().contains("nao.kdl already exists"));
        assert_eq!(
            pal.read_file_string("nao.kdl").as_deref(),
            Some("recipe \"existing\" {}")
        );
    }

    #[test]
    fn run_with_version_returns_success() {
        let pal = PalMock::new();
        let flags = Nao::from_vec(vec![OsString::from("--version")]).unwrap();

        let exit_code = run_with_pal_and_version_loader(flags, PalHandle::new(pal), || {
            Ok(VersionMetadata {
                last_commit_date: SharedString::from("2026-03-21"),
                short_commit_id: SharedString::from("abc1234"),
                has_uncommitted_changes: true,
            })
        })
        .unwrap();

        assert_eq!(exit_code, ExitCode::SUCCESS);
    }
}
