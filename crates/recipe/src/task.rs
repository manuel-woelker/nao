use crate::artifact_spec::ArtifactSpec;
use crate::dependency_name::DependencyName;
use crate::environment_spec::EnvironmentSpec;
use crate::run_spec::RunSpec;
use crate::task_name::TaskName;

/// Represents a task in a recipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    /// Task name.
    pub name: TaskName,
    /// Named prerequisite tasks.
    pub dependencies: Vec<DependencyName>,
    /// Task execution configuration.
    pub run: RunSpec,
    /// Environment variables for the task.
    pub environment: Vec<EnvironmentSpec>,
    /// Declared task artifacts.
    pub artifacts: Vec<ArtifactSpec>,
}
