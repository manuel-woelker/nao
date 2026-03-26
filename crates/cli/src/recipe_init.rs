use nao_base::err;
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_pal::pal::Pal;

pub(crate) fn initialize_recipe_file(pal: &dyn Pal, path: &FilePath) -> NaoResult<()> {
    if pal.file_exists(path)? {
        return Err(err!("{path} already exists"));
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
    use expect_test::expect;
    use nao_base::file_path::FilePath;
    use nao_pal::pal_mock::PalMock;

    #[test]
    fn init_writes_starter_recipe_when_missing() {
        let pal = PalMock::new();

        initialize_recipe_file(&pal, &FilePath::from("nao.kdl")).unwrap();

        expect![[r#"
            WRITE FILE: nao.kdl -> recipe "default" {
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
            pal.read_file_string("nao.kdl").as_deref(),
            Some(starter_recipe())
        );
    }

    #[test]
    fn init_keeps_existing_recipe_file() {
        let pal = PalMock::new();
        pal.set_file("nao.kdl", "recipe \"existing\" {}");

        let error = initialize_recipe_file(&pal, &FilePath::from("nao.kdl")).unwrap_err();

        assert!(error.to_test_string().contains("nao.kdl already exists"));
        assert_eq!(pal.get_effects(), "");
        assert_eq!(
            pal.read_file_string("nao.kdl").as_deref(),
            Some("recipe \"existing\" {}")
        );
    }
}
