mod runner;

use nao_base::err;
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_pal::pal_real::PalReal;
use runner::Runner;
use std::path::PathBuf;
use std::process::ExitCode;

xflags::xflags! {
    cmd nao {
        optional --list
        optional --tui
        optional --config config: PathBuf
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
    let flags = Nao::from_env().map_err(|error| err!("{error}"))?;

    if should_run_tui(&flags) {
        validate_tui_request(&flags)?;
        let recipe_path = flags
            .config
            .clone()
            .unwrap_or_else(|| PathBuf::from("nao.kdl"));
        nao_tui::run(PalReal::new_handle(), FilePath::new(&recipe_path))?;
        return Ok(ExitCode::SUCCESS);
    }

    let recipe_path = flags.config.unwrap_or_else(|| PathBuf::from("nao.kdl"));
    let runner = Runner::new(PalReal::new_handle());
    let output = runner.execute(&FilePath::new(&recipe_path), flags.list, &flags.task_name)?;

    print!("{}", output.output);
    Ok(output.exit_code)
}

fn should_run_tui(flags: &Nao) -> bool {
    flags.tui || (!flags.list && flags.task_name.is_empty())
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

#[cfg(test)]
mod tests {
    use super::Nao;
    use super::should_run_tui;
    use super::validate_tui_request;
    use std::ffi::OsString;
    use std::path::PathBuf;

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
}
