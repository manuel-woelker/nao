use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_pal::pal::Pal;

pub(crate) const PRIMARY_RECIPE_PATH: &str = ".nao/nao.kdl";
pub(crate) const LEGACY_RECIPE_PATH: &str = "nao.kdl";
pub(crate) const PRIMARY_RECIPE_GITIGNORE_PATH: &str = ".nao/.gitignore";
pub(crate) const PRIMARY_RECIPE_GITIGNORE_CONTENT: &str = "*\n!.gitignore\n!nao.kdl\n";

pub(crate) fn primary_recipe_path() -> FilePath {
    FilePath::from(PRIMARY_RECIPE_PATH)
}

pub(crate) fn legacy_recipe_path() -> FilePath {
    FilePath::from(LEGACY_RECIPE_PATH)
}

pub(crate) fn primary_recipe_gitignore_path() -> FilePath {
    FilePath::from(PRIMARY_RECIPE_GITIGNORE_PATH)
}

pub(crate) fn resolve_default_recipe_path(pal: &dyn Pal) -> NaoResult<FilePath> {
    let primary = primary_recipe_path();
    if pal.file_exists(&primary)? {
        return Ok(primary);
    }

    let legacy = legacy_recipe_path();
    if pal.file_exists(&legacy)? {
        return Ok(legacy);
    }

    Ok(primary)
}

#[cfg(test)]
mod tests {
    use super::PRIMARY_RECIPE_PATH;
    use super::legacy_recipe_path;
    use super::primary_recipe_path;
    use super::resolve_default_recipe_path;
    use nao_pal::pal_mock::PalMock;

    #[test]
    fn prefers_primary_recipe_path_when_present() {
        let pal = PalMock::new();
        pal.set_file(PRIMARY_RECIPE_PATH, "recipe \"default\" {}");
        pal.set_file("nao.kdl", "recipe \"legacy\" {}");

        let recipe_path = resolve_default_recipe_path(&pal).unwrap();

        assert_eq!(recipe_path, primary_recipe_path());
    }

    #[test]
    fn falls_back_to_legacy_recipe_path_when_primary_is_missing() {
        let pal = PalMock::new();
        pal.set_file("nao.kdl", "recipe \"legacy\" {}");

        let recipe_path = resolve_default_recipe_path(&pal).unwrap();

        assert_eq!(recipe_path, legacy_recipe_path());
    }

    #[test]
    fn defaults_to_primary_recipe_path_when_no_recipe_exists() {
        let pal = PalMock::new();

        let recipe_path = resolve_default_recipe_path(&pal).unwrap();

        assert_eq!(recipe_path, primary_recipe_path());
    }
}
