mod runner;

use nao_base::err;
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_pal::pal::Pal;
use nao_pal::pal::PalHandle;
use nao_pal::pal_real::PalReal;
use runner::Runner;
use std::path::PathBuf;
use std::process::ExitCode;

xflags::xflags! {
    cmd nao {
        optional --init
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
    let pal = PalReal::new_handle();

    run_with_pal(flags, pal)
}

fn run_with_pal(flags: Nao, pal: PalHandle) -> NaoResult<ExitCode> {
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
    flags.tui || (!flags.init && !flags.list && flags.task_name.is_empty())
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

#[cfg(test)]
mod tests {
    use super::Nao;
    use super::initialize_recipe_file;
    use super::run_with_pal;
    use super::should_run_tui;
    use super::starter_recipe;
    use super::validate_init_request;
    use super::validate_tui_request;
    use expect_test::expect;
    use nao_base::file_path::FilePath;
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

        let exit_code = run_with_pal(flags, PalHandle::new(pal.clone())).unwrap();

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

        let error = run_with_pal(flags, PalHandle::new(pal.clone())).unwrap_err();

        assert!(error.to_test_string().contains("nao.kdl already exists"));
        assert_eq!(
            pal.read_file_string("nao.kdl").as_deref(),
            Some("recipe \"existing\" {}")
        );
    }
}
