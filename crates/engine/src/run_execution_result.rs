use nao_base::file_path::FilePath;
use nao_base::shared_string::SharedString;

/// Describes the rendered result of executing a planned run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunExecutionResult {
    /// User-facing rendered output for the executed run.
    pub output: SharedString,
    /// Directory that stores the run artifacts.
    pub run_directory: FilePath,
}
