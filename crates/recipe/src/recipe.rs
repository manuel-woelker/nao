use crate::recipe_config::RecipeConfig;
use crate::task::Task;
use nao_base::shared_string::SharedString;

/// Represents a parsed recipe file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipe {
    /// Recipe name.
    pub name: SharedString,
    /// Recipe-wide configuration values.
    pub config: RecipeConfig,
    /// Tasks contained in the recipe.
    pub tasks: Vec<Task>,
}
