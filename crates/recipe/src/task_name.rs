use nao_base::shared_string::SharedString;

/// Names a task in a recipe.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskName(pub SharedString);

impl TaskName {
    /// Returns the task name as a string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<SharedString> for TaskName {
    fn from(value: SharedString) -> Self {
        Self(value)
    }
}

impl From<&str> for TaskName {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}
