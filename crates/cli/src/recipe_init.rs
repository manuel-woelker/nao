use crate::recipe_paths::PRIMARY_RECIPE_GITIGNORE_CONTENT;
use crate::recipe_paths::legacy_recipe_path;
use crate::recipe_paths::primary_recipe_gitignore_path;
use nao_base::err;
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_pal::pal::Pal;

pub(crate) fn initialize_recipe_file(pal: &dyn Pal, path: &FilePath) -> NaoResult<()> {
    let legacy_path = legacy_recipe_path();
    if pal.file_exists(path)? {
        return Err(err!("{path} already exists"));
    }
    if legacy_path != *path && pal.file_exists(&legacy_path)? {
        return Err(err!("{legacy_path} already exists"));
    }

    let recipe_directory = path.parent().unwrap_or_else(|| FilePath::from("."));
    pal.create_directory_all(&recipe_directory)?;
    let gitignore_path = primary_recipe_gitignore_path();
    if gitignore_path.parent() == Some(recipe_directory.clone())
        && !pal.file_exists(&gitignore_path)?
    {
        pal.write_file(&gitignore_path, PRIMARY_RECIPE_GITIGNORE_CONTENT.as_bytes())?;
    }
    pal.write_file(path, starter_recipe().as_bytes())?;
    println!("Created {path}");
    Ok(())
}

pub(crate) fn starter_recipe() -> &'static str {
    r#"recipe "default" {
  task "build" description="Sample build task using direct outcome output" {
    run shell="""
      printf 'Building the project...\n'
      printf 'Task outcome: build artifacts are ready\n'
    """
  }

  task "test" description="Sample test task with an explicit outcome line" {
    depends-on "build"
    run shell="""
      printf 'Running sample tests...\n'
      printf 'Task outcome: 3 sample tests passed\n'
    """
  }
}
"#
}

#[cfg(test)]
mod tests {
    use super::initialize_recipe_file;
    use super::starter_recipe;
    use crate::recipe_paths::PRIMARY_RECIPE_GITIGNORE_CONTENT;
    use expect_test::expect;
    use nao_base::file_path::FilePath;
    use nao_pal::pal_mock::PalMock;

    #[test]
    fn init_writes_starter_recipe_when_missing() {
        let pal = PalMock::new();

        initialize_recipe_file(&pal, &FilePath::from(".nao/nao.kdl")).unwrap();

        expect![[r#"
            CREATE DIRECTORY: .nao
            WRITE FILE: .nao/.gitignore -> *
            !.gitignore
            !nao.kdl

            WRITE FILE: .nao/nao.kdl -> recipe "default" {
              task "build" description="Sample build task using direct outcome output" {
                run shell="""
                  printf 'Building the project...\n'
                  printf 'Task outcome: build artifacts are ready\n'
                """
              }

              task "test" description="Sample test task with an explicit outcome line" {
                depends-on "build"
                run shell="""
                  printf 'Running sample tests...\n'
                  printf 'Task outcome: 3 sample tests passed\n'
                """
              }
            }

        "#]]
        .assert_eq(&pal.get_effects());
        assert_eq!(
            pal.read_file_string(".nao/nao.kdl").as_deref(),
            Some(starter_recipe())
        );
        assert_eq!(
            pal.read_file_string(".nao/.gitignore").as_deref(),
            Some(PRIMARY_RECIPE_GITIGNORE_CONTENT)
        );
    }

    #[test]
    fn init_keeps_existing_recipe_file() {
        let pal = PalMock::new();
        pal.set_file(".nao/nao.kdl", "recipe \"existing\" {}");

        let error = initialize_recipe_file(&pal, &FilePath::from(".nao/nao.kdl")).unwrap_err();

        assert!(
            error
                .to_test_string()
                .contains(".nao/nao.kdl already exists")
        );
        assert_eq!(pal.get_effects(), "");
        assert_eq!(
            pal.read_file_string(".nao/nao.kdl").as_deref(),
            Some("recipe \"existing\" {}")
        );
    }

    #[test]
    fn init_refuses_to_create_primary_recipe_when_legacy_recipe_exists() {
        let pal = PalMock::new();
        pal.set_file("nao.kdl", "recipe \"existing\" {}");

        let error = initialize_recipe_file(&pal, &FilePath::from(".nao/nao.kdl")).unwrap_err();

        assert!(error.to_test_string().contains("nao.kdl already exists"));
        assert_eq!(pal.get_effects(), "");
    }
}
