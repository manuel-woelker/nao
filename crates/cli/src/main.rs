mod command_dispatch;
mod help_text;
mod recipe_init;
mod recipe_paths;
mod request_validation;
mod restart_marker;
mod runner;
mod version_metadata;

use std::path::PathBuf;
use std::process::ExitCode;

shadow_rs::shadow!(build);

xflags::xflags! {
    /// Run local task graphs defined in a `nao.kdl` recipe file.
    cmd nao {
        /// Create a starter `.nao/nao.kdl` in the current directory.
        optional --init
        /// List task names from the selected recipe file.
        optional --list
        /// Open the terminal UI.
        optional --tui
        /// Run with CI-friendly logging and a final task-log summary.
        optional --ci
        /// Print build-time version metadata.
        optional --version
        /// Request restart of a currently running nao process in this workspace.
        optional --restart
        /// Load a recipe file other than the default `.nao/nao.kdl`.
        optional --config config: PathBuf
        /// Task names or wildcard selectors to execute.
        repeated task_name: String
    }
}

fn main() -> ExitCode {
    command_dispatch::main()
}

#[cfg(test)]
mod tests {
    use crate::Nao;
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
    fn parses_ci_flag() {
        let flags = Nao::from_vec(vec![OsString::from("--ci"), OsString::from("build")]).unwrap();

        assert!(flags.ci);
        assert_eq!(flags.task_name, vec!["build".to_owned()]);
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
        assert!(!flags.restart);
        assert!(!flags.init);
        assert!(!flags.list);
        assert!(!flags.tui);
        assert_eq!(flags.config, None);
        assert!(flags.task_name.is_empty());
    }

    #[test]
    fn parses_restart_flag() {
        let flags = Nao::from_vec(vec![OsString::from("--restart")]).unwrap();

        assert!(flags.restart);
        assert!(!flags.version);
        assert!(!flags.init);
        assert!(!flags.list);
        assert!(!flags.tui);
        assert_eq!(flags.config, None);
        assert!(flags.task_name.is_empty());
    }
}
