use nao_base::shared_string::SharedString;
use std::error::Error;
use std::fmt::{Display, Formatter};

/// Describes a recipe parse or validation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeError {
    message: SharedString,
}

impl RecipeError {
    /// Creates a new recipe error with a human-readable message.
    pub fn new(message: impl Into<SharedString>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Returns the error message.
    pub fn message(&self) -> &str {
        self.message.as_str()
    }
}

impl Display for RecipeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl Error for RecipeError {}
