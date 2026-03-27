use nao_base::shared_string::SharedString;

/// Controls how the scheduler reacts after a task failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureMode {
    /// Stop launching new tasks after the first failure.
    FailEarly,
    /// Continue launching tasks that do not depend on failed work.
    FailLate,
}

impl FailureMode {
    /// Parses a recipe config value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "fail-early" => Some(Self::FailEarly),
            "fail-late" => Some(Self::FailLate),
            _ => None,
        }
    }

    /// Returns the config string representation.
    pub fn as_str(self) -> SharedString {
        match self {
            Self::FailEarly => SharedString::from("fail-early"),
            Self::FailLate => SharedString::from("fail-late"),
        }
    }
}
