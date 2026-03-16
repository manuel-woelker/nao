//! Core types for task recipe definitions.

/// Returns the crate name.
pub fn crate_name() -> &'static str {
    "nao-recipe"
}

#[cfg(test)]
mod tests {
    use super::crate_name;

    #[test]
    fn returns_crate_name() {
        assert_eq!(crate_name(), "nao-recipe");
    }
}
