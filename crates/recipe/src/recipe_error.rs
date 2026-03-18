use nao_base::shared_string::SharedString;
use std::error::Error;
use std::fmt::{Display, Formatter};

/// Describes a recipe parse or validation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeError {
    message: SharedString,
    rendered_location: Option<SharedString>,
}

impl RecipeError {
    /// Creates a new recipe error with a human-readable message.
    pub fn new(message: impl Into<SharedString>) -> Self {
        Self {
            message: message.into(),
            rendered_location: None,
        }
    }

    /// Creates a new recipe error with an attached source-location rendering.
    pub fn with_rendered_location(
        message: impl Into<SharedString>,
        rendered_location: impl Into<SharedString>,
    ) -> Self {
        Self {
            message: message.into(),
            rendered_location: Some(rendered_location.into()),
        }
    }

    /// Returns the error message.
    pub fn message(&self) -> &str {
        self.message.as_str()
    }
}

impl Display for RecipeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())?;

        if let Some(rendered_location) = &self.rendered_location {
            f.write_str("\n")?;
            f.write_str(rendered_location.as_str())?;
        }

        Ok(())
    }
}

impl Error for RecipeError {}
