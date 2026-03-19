/// Controls how live task execution is rendered in interactive terminals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveDisplay {
    /// Show one aggregate status line for the whole run.
    SingleLine,
    /// Show one continuously updated line per planned task.
    LinePerTask,
}

impl LiveDisplay {
    /// Parses a config string into a live display mode.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "single-line" => Some(Self::SingleLine),
            "line-per-task" => Some(Self::LinePerTask),
            _ => None,
        }
    }

    /// Returns the config string for this live display mode.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SingleLine => "single-line",
            Self::LinePerTask => "line-per-task",
        }
    }
}
