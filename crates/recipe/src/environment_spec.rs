use nao_base::shared_string::SharedString;

/// Defines a single environment variable for task execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentSpec {
    /// Environment variable name.
    pub name: SharedString,
    /// Environment variable value.
    pub value: SharedString,
}
