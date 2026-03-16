use nao_base::file_path::FilePath;
use nao_base::shared_string::SharedString;

/// Describes an artifact produced by a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactSpec {
    /// Logical artifact name.
    pub name: SharedString,
    /// Output path for the artifact.
    pub path: FilePath,
}
