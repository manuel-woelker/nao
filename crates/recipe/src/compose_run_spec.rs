use nao_base::file_path::FilePath;
use nao_base::shared_string::SharedString;

/// Describes Docker Compose-based execution details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeRunSpec {
    /// Compose project directory.
    pub directory: FilePath,
    /// Compose service name to run.
    pub service: SharedString,
    /// Positional service arguments.
    pub args: Vec<SharedString>,
}
