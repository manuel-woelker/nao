use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_pal::pal::Pal;
use std::time::SystemTime;

const RESTART_MARKER_DIRECTORY: &str = ".nao/internal";
const RESTART_MARKER_PATH: &str = ".nao/internal/restart";

pub(crate) fn ensure_restart_marker(pal: &dyn Pal) -> NaoResult<SystemTime> {
    let directory = restart_marker_directory();
    pal.create_directory_all(&directory)?;
    let path = restart_marker_path();
    if !pal.file_exists(&path)? {
        pal.write_file(&path, b"")?;
    }
    pal.file_modified_time(&path)
}

pub(crate) fn restart_marker_modified_time(pal: &dyn Pal) -> NaoResult<SystemTime> {
    pal.file_modified_time(&restart_marker_path())
}

pub(crate) fn touch_restart_marker(pal: &dyn Pal) -> NaoResult<()> {
    pal.create_directory_all(&restart_marker_directory())?;
    pal.touch_file(&restart_marker_path())
}

fn restart_marker_directory() -> FilePath {
    FilePath::from(RESTART_MARKER_DIRECTORY)
}

fn restart_marker_path() -> FilePath {
    FilePath::from(RESTART_MARKER_PATH)
}

#[cfg(test)]
mod tests {
    use super::ensure_restart_marker;
    use super::restart_marker_modified_time;
    use super::touch_restart_marker;
    use nao_pal::pal_mock::PalMock;
    use std::time::Duration;
    use std::time::SystemTime;

    #[test]
    fn startup_creates_missing_restart_marker() {
        let pal = PalMock::new();
        pal.set_current_system_time(SystemTime::UNIX_EPOCH + Duration::from_secs(1));

        let modified_time = ensure_restart_marker(&pal).unwrap();

        assert_eq!(
            modified_time,
            SystemTime::UNIX_EPOCH + Duration::from_secs(1)
        );
        assert_eq!(
            pal.read_file_bytes(".nao/internal/restart"),
            Some(Vec::new())
        );
        assert!(
            pal.get_effects()
                .contains("CREATE DIRECTORY: .nao/internal")
        );
        assert!(
            pal.get_effects()
                .contains("WRITE FILE: .nao/internal/restart -> ")
        );
    }

    #[test]
    fn startup_preserves_existing_restart_marker_mtime() {
        let pal = PalMock::new();
        let original_time = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
        pal.set_current_system_time(original_time);
        pal.set_file(".nao/internal/restart", "");
        pal.clear_effects();
        pal.set_current_system_time(SystemTime::UNIX_EPOCH + Duration::from_secs(2));

        let modified_time = ensure_restart_marker(&pal).unwrap();

        assert_eq!(modified_time, original_time);
        assert!(
            !pal.get_effects()
                .contains("WRITE FILE: .nao/internal/restart")
        );
    }

    #[test]
    fn restart_command_touches_marker() {
        let pal = PalMock::new();
        pal.set_current_system_time(SystemTime::UNIX_EPOCH + Duration::from_secs(1));
        ensure_restart_marker(&pal).unwrap();
        pal.set_current_system_time(SystemTime::UNIX_EPOCH + Duration::from_secs(2));

        touch_restart_marker(&pal).unwrap();

        assert_eq!(
            restart_marker_modified_time(&pal).unwrap(),
            SystemTime::UNIX_EPOCH + Duration::from_secs(2)
        );
        assert!(
            pal.get_effects()
                .contains("TOUCH FILE: .nao/internal/restart")
        );
    }
}
