use crate::Nao;
use crate::help_text::render_help;
use crate::help_text::render_non_interactive_default_help;
use crate::recipe_init::initialize_recipe_file;
use crate::request_validation::is_default_action_request;
use crate::request_validation::should_run_tui;
use crate::request_validation::validate_ci_request;
use crate::request_validation::validate_init_request;
use crate::request_validation::validate_tui_request;
use crate::request_validation::validate_version_request;
use crate::runner::Runner;
use crate::version_metadata::VersionMetadata;
use crate::version_metadata::load_version_metadata;
use crate::version_metadata::render_version;
use nao_base::err;
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_pal::pal::PalHandle;
use nao_pal::pal_real::PalReal;
use std::path::PathBuf;
use std::process::ExitCode;

pub(crate) fn main() -> ExitCode {
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

pub(crate) fn run() -> NaoResult<ExitCode> {
    let raw_args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let flags = match Nao::from_vec(raw_args) {
        Ok(flags) => flags,
        Err(error) if error.is_help() => {
            print!("{}", render_help(&error.to_string()));
            return Ok(ExitCode::SUCCESS);
        }
        Err(error) => return Err(err!("{error}")),
    };
    let pal = PalReal::new_handle()?;

    run_with_pal_and_version_loader(flags, pal, load_version_metadata)
}

pub(crate) fn run_with_pal_and_version_loader<F>(
    flags: Nao,
    pal: PalHandle,
    load_version_metadata: F,
) -> NaoResult<ExitCode>
where
    F: FnOnce() -> NaoResult<VersionMetadata>,
{
    let interactive_terminal = pal.is_interactive_terminal();

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

    validate_ci_request(&flags)?;

    if is_default_action_request(&flags) && !interactive_terminal {
        print!("{}", render_non_interactive_default_help(Nao::HELP_));
        return Ok(ExitCode::SUCCESS);
    }

    if should_run_tui(&flags, interactive_terminal) {
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
    let output = runner.execute(
        &FilePath::new(&recipe_path),
        flags.list,
        flags.ci,
        &flags.task_name,
    )?;

    print!("{}", output.output);
    Ok(output.exit_code)
}

#[cfg(test)]
mod tests {
    use super::run_with_pal_and_version_loader;
    use crate::Nao;
    use crate::recipe_init::starter_recipe;
    use crate::version_metadata::VersionMetadata;
    use nao_base::file_path::FilePath;
    use nao_base::shared_string::SharedString;
    use nao_base::timestamp::Timestamp;
    use nao_pal::pal::PalHandle;
    use nao_pal::pal_mock::PalMock;
    use nao_pal::process_command::ProcessCommand;
    use nao_pal::process_event::ProcessEvent;
    use nao_pal::process_exited_event::ProcessExitedEvent;
    use nao_pal::process_result::ProcessResult;
    use nao_pal::process_stream_closed_event::ProcessStreamClosedEvent;
    use std::ffi::OsString;
    use std::process::ExitCode;

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

    #[test]
    fn run_returns_exit_code_one_when_task_fails_with_non_one_status() {
        let pal = PalMock::new();
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              task "test" {
                run script="./scripts/test.sh"
              }
            }
            "#,
        );
        pal.set_process_execution(
            ProcessCommand {
                executable: "./scripts/test.sh".into(),
                arguments: Vec::new(),
                working_directory: Some(FilePath::from(".")),
                environment: Vec::new(),
            },
            vec![
                ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                    timestamp: Timestamp::new(1),
                    stream: nao_pal::process_output_stream::ProcessOutputStream::Stdout,
                }),
                ProcessEvent::StreamClosed(ProcessStreamClosedEvent {
                    timestamp: Timestamp::new(2),
                    stream: nao_pal::process_output_stream::ProcessOutputStream::Stderr,
                }),
                ProcessEvent::Exited(ProcessExitedEvent {
                    timestamp: Timestamp::new(3),
                    exit_code: Some(5),
                }),
            ],
            ProcessResult {
                started_at: Timestamp::new(0),
                finished_at: Timestamp::new(3),
                exit_code: Some(5),
            },
        );
        let flags = Nao::from_vec(vec![OsString::from("test")]).unwrap();

        let exit_code = run_with_pal_and_version_loader(flags, PalHandle::new(pal), || {
            unreachable!("task execution should not load version metadata")
        })
        .unwrap();

        assert_eq!(exit_code, ExitCode::from(1));
    }

    #[test]
    fn run_without_action_in_non_interactive_mode_returns_success_without_reading_recipe() {
        let pal = PalMock::new();
        let flags = Nao::from_vec(Vec::<OsString>::new()).unwrap();

        let exit_code = run_with_pal_and_version_loader(flags, PalHandle::new(pal.clone()), || {
            unreachable!("default non-interactive help should not load version metadata")
        })
        .unwrap();

        assert_eq!(exit_code, ExitCode::SUCCESS);
        pal.verify_effects(expect_test::expect![""]);
    }
}
