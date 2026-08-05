use crate::Nao;
use nao_base::err;
use nao_base::result::NaoResult;

pub(crate) fn is_default_action_request(flags: &Nao) -> bool {
    !flags.version
        && !flags.init
        && !flags.list
        && !flags.restart
        && flags.task_name.is_empty()
        && !flags.ci
}

pub(crate) fn should_run_tui(flags: &Nao, interactive_terminal: bool) -> bool {
    (flags.tui || (interactive_terminal && is_default_action_request(flags))) && !flags.ci
}

pub(crate) fn validate_tui_request(flags: &Nao) -> NaoResult<()> {
    if flags.list {
        return Err(err!("--tui cannot be combined with --list"));
    }
    if !flags.task_name.is_empty() {
        return Err(err!("--tui cannot be combined with task names"));
    }
    Ok(())
}

pub(crate) fn validate_ci_request(flags: &Nao) -> NaoResult<()> {
    if flags.ci && flags.tui {
        return Err(err!("--ci cannot be combined with --tui"));
    }
    Ok(())
}

pub(crate) fn validate_restart_request(flags: &Nao) -> NaoResult<()> {
    if flags.init {
        return Err(err!("--restart cannot be combined with --init"));
    }
    if flags.list {
        return Err(err!("--restart cannot be combined with --list"));
    }
    if flags.tui {
        return Err(err!("--restart cannot be combined with --tui"));
    }
    if flags.ci {
        return Err(err!("--restart cannot be combined with --ci"));
    }
    if flags.version {
        return Err(err!("--restart cannot be combined with --version"));
    }
    if flags.config.is_some() {
        return Err(err!("--restart cannot be combined with --config"));
    }
    if !flags.task_name.is_empty() {
        return Err(err!("--restart cannot be combined with task names"));
    }
    Ok(())
}

pub(crate) fn validate_version_request(flags: &Nao) -> NaoResult<()> {
    if flags.init {
        return Err(err!("--version cannot be combined with --init"));
    }
    if flags.restart {
        return Err(err!("--version cannot be combined with --restart"));
    }
    if flags.list {
        return Err(err!("--version cannot be combined with --list"));
    }
    if flags.tui {
        return Err(err!("--version cannot be combined with --tui"));
    }
    if flags.ci {
        return Err(err!("--version cannot be combined with --ci"));
    }
    if flags.config.is_some() {
        return Err(err!("--version cannot be combined with --config"));
    }
    if !flags.task_name.is_empty() {
        return Err(err!("--version cannot be combined with task names"));
    }
    Ok(())
}

pub(crate) fn validate_init_request(flags: &Nao) -> NaoResult<()> {
    if flags.restart {
        return Err(err!("--init cannot be combined with --restart"));
    }
    if flags.list {
        return Err(err!("--init cannot be combined with --list"));
    }
    if flags.tui {
        return Err(err!("--init cannot be combined with --tui"));
    }
    if flags.ci {
        return Err(err!("--init cannot be combined with --ci"));
    }
    if flags.config.is_some() {
        return Err(err!("--init cannot be combined with --config"));
    }
    if !flags.task_name.is_empty() {
        return Err(err!("--init cannot be combined with task names"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_default_action_request;
    use super::should_run_tui;
    use super::validate_ci_request;
    use super::validate_init_request;
    use super::validate_restart_request;
    use super::validate_tui_request;
    use super::validate_version_request;
    use crate::Nao;
    use std::ffi::OsString;

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
    fn rejects_tui_with_ci() {
        let flags = Nao::from_vec(vec![OsString::from("--tui"), OsString::from("--ci")]).unwrap();

        let error = validate_ci_request(&flags).unwrap_err();

        assert!(
            error
                .to_test_string()
                .contains("--ci cannot be combined with --tui")
        );
    }

    #[test]
    fn rejects_task_names_with_restart() {
        let flags =
            Nao::from_vec(vec![OsString::from("--restart"), OsString::from("build")]).unwrap();

        let error = validate_restart_request(&flags).unwrap_err();

        assert!(
            error
                .to_test_string()
                .contains("--restart cannot be combined with task names")
        );
    }

    #[test]
    fn rejects_config_with_restart() {
        let flags = Nao::from_vec(vec![
            OsString::from("--restart"),
            OsString::from("--config"),
            OsString::from("custom.kdl"),
        ])
        .unwrap();

        let error = validate_restart_request(&flags).unwrap_err();

        assert!(
            error
                .to_test_string()
                .contains("--restart cannot be combined with --config")
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
    fn rejects_restart_with_version() {
        let flags = Nao::from_vec(vec![
            OsString::from("--version"),
            OsString::from("--restart"),
        ])
        .unwrap();

        let error = validate_version_request(&flags).unwrap_err();

        assert!(
            error
                .to_test_string()
                .contains("--version cannot be combined with --restart")
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
    fn rejects_restart_with_init() {
        let flags =
            Nao::from_vec(vec![OsString::from("--init"), OsString::from("--restart")]).unwrap();

        let error = validate_init_request(&flags).unwrap_err();

        assert!(
            error
                .to_test_string()
                .contains("--init cannot be combined with --restart")
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

        assert!(is_default_action_request(&flags));
        assert!(should_run_tui(&flags, true));
    }

    #[test]
    fn does_not_default_to_tui_when_no_action_is_given_in_non_interactive_mode() {
        let flags = Nao::from_vec(Vec::<OsString>::new()).unwrap();

        assert!(is_default_action_request(&flags));
        assert!(!should_run_tui(&flags, false));
    }

    #[test]
    fn does_not_default_to_tui_when_listing_tasks() {
        let flags = Nao::from_vec(vec![OsString::from("--list")]).unwrap();

        assert!(!should_run_tui(&flags, true));
    }

    #[test]
    fn does_not_default_to_tui_when_tasks_are_requested() {
        let flags = Nao::from_vec(vec![OsString::from("build")]).unwrap();

        assert!(!should_run_tui(&flags, true));
    }

    #[test]
    fn does_not_default_to_tui_when_init_is_requested() {
        let flags = Nao::from_vec(vec![OsString::from("--init")]).unwrap();

        assert!(!should_run_tui(&flags, true));
    }

    #[test]
    fn does_not_default_to_tui_when_version_is_requested() {
        let flags = Nao::from_vec(vec![OsString::from("--version")]).unwrap();

        assert!(!should_run_tui(&flags, true));
    }

    #[test]
    fn does_not_default_to_tui_when_ci_is_requested() {
        let flags = Nao::from_vec(vec![OsString::from("--ci")]).unwrap();

        assert!(!should_run_tui(&flags, true));
    }
}
