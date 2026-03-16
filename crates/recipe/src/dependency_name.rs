use nao_base::shared_string::SharedString;

/// Names a prerequisite task.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DependencyName(pub SharedString);

impl DependencyName {
    /// Returns the dependency name as a string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<SharedString> for DependencyName {
    fn from(value: SharedString) -> Self {
        Self(value)
    }
}

impl From<&str> for DependencyName {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}
