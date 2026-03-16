use nao_base::file_path::FilePath;
use nao_base::shared_string::SharedString;

/// Describes container-based execution details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerRunSpec {
    /// Container image reference.
    pub image: SharedString,
    /// Positional container arguments.
    pub args: Vec<SharedString>,
}

/// Describes how a task should execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunSpec {
    /// Executes a shell command.
    Shell(SharedString),
    /// Executes a script path.
    Script(FilePath),
    /// Executes a container image.
    Container(ContainerRunSpec),
}
