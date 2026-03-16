use crate::artifact_spec::ArtifactSpec;
use crate::dependency_name::DependencyName;
use crate::environment_spec::EnvironmentSpec;
use crate::recipe::Recipe;
use crate::recipe_error::RecipeError;
use crate::run_spec::{ContainerRunSpec, RunSpec};
use crate::task::Task;
use crate::task_name::TaskName;
use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};
use nao_base::file_path::FilePath;
use nao_base::result::{NaoResult, ResultExt};
use nao_base::shared_string::SharedString;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Parses a recipe from a KDL source string.
pub fn parse_recipe(source: &str) -> Result<Recipe, RecipeError> {
    let document: KdlDocument = source
        .parse()
        .map_err(|error| RecipeError::new(format!("failed to parse recipe KDL: {error}")))?;
    parse_recipe_document(&document)
}

/// Loads and parses a recipe file from disk.
pub fn load_recipe(path: impl AsRef<Path>) -> NaoResult<Recipe> {
    let path = path.as_ref();
    let source = fs::read_to_string(path)
        .with_context(|| format!("failed to read recipe file {}", path.display()))?;
    parse_recipe(&source).map_err(Into::into)
}

fn parse_recipe_document(document: &KdlDocument) -> Result<Recipe, RecipeError> {
    let nodes = document.nodes();
    if nodes.len() != 1 {
        return Err(RecipeError::new(format!(
            "expected exactly one top-level recipe node, found {}",
            nodes.len()
        )));
    }

    let recipe_node = &nodes[0];
    expect_node_name(recipe_node, "recipe")?;
    let recipe_name = parse_required_string_argument(recipe_node, "recipe name")?;
    let children = recipe_node
        .children()
        .ok_or_else(|| RecipeError::new("recipe node must contain task definitions"))?;

    let mut tasks = Vec::new();
    for child in children.nodes() {
        expect_node_name(child, "task")?;
        tasks.push(parse_task(child)?);
    }

    validate_tasks(&tasks)?;

    Ok(Recipe {
        name: recipe_name,
        tasks,
    })
}

fn parse_task(node: &KdlNode) -> Result<Task, RecipeError> {
    let name = TaskName(parse_required_string_argument(node, "task name")?);
    let children = node.children().ok_or_else(|| {
        RecipeError::new(format!("task `{}` must have child nodes", name.as_str()))
    })?;

    let mut dependencies = Vec::new();
    let mut run = None;
    let mut environment = Vec::new();
    let mut artifacts = Vec::new();

    for child in children.nodes() {
        match child.name().value() {
            "depends-on" => {
                dependencies.push(DependencyName(parse_required_string_argument(
                    child,
                    "dependency name",
                )?));
            }
            "run" => {
                if run.is_some() {
                    return Err(RecipeError::new(format!(
                        "task `{}` cannot define multiple run nodes",
                        name.as_str()
                    )));
                }
                run = Some(parse_run(child, name.as_str())?);
            }
            "env" => {
                environment.push(parse_environment(child, name.as_str())?);
            }
            "artifact" => {
                artifacts.push(parse_artifact(child, name.as_str())?);
            }
            other => {
                return Err(RecipeError::new(format!(
                    "task `{}` contains unsupported node `{other}`",
                    name.as_str()
                )));
            }
        }
    }

    let run = run.ok_or_else(|| {
        RecipeError::new(format!("task `{}` is missing a run node", name.as_str()))
    })?;

    Ok(Task {
        name,
        dependencies,
        run,
        environment,
        artifacts,
    })
}

fn parse_run(node: &KdlNode, task_name: &str) -> Result<RunSpec, RecipeError> {
    let shell = parse_optional_string_property(node, "shell")?;
    let script = parse_optional_string_property(node, "script")?;
    let container = parse_optional_string_property(node, "container")?;

    let defined = [shell.is_some(), script.is_some(), container.is_some()]
        .into_iter()
        .filter(|present| *present)
        .count();

    if defined != 1 {
        return Err(RecipeError::new(format!(
            "task `{task_name}` run node must define exactly one of `shell`, `script`, or `container`"
        )));
    }

    if let Some(shell) = shell {
        ensure_no_children(node, task_name, "shell")?;
        return Ok(RunSpec::Shell(shell));
    }

    if let Some(script) = script {
        ensure_no_children(node, task_name, "script")?;
        return Ok(RunSpec::Script(FilePath::from(script)));
    }

    let args = parse_container_args(node, task_name)?;
    Ok(RunSpec::Container(ContainerRunSpec {
        image: container.expect("container property checked above"),
        args,
    }))
}

fn parse_container_args(node: &KdlNode, task_name: &str) -> Result<Vec<SharedString>, RecipeError> {
    let Some(children) = node.children() else {
        return Ok(Vec::new());
    };

    let mut args = Vec::new();
    for child in children.nodes() {
        if child.name().value() != "args" {
            return Err(RecipeError::new(format!(
                "task `{task_name}` container run only supports `args` child nodes"
            )));
        }

        for entry in child.entries() {
            if entry.name().is_some() {
                return Err(RecipeError::new(format!(
                    "task `{task_name}` container args must use positional string values"
                )));
            }
            args.push(parse_string_value(entry, "container argument")?);
        }
    }

    Ok(args)
}

fn parse_environment(node: &KdlNode, task_name: &str) -> Result<EnvironmentSpec, RecipeError> {
    if !node.entries().iter().all(|entry| entry.name().is_some()) {
        return Err(RecipeError::new(format!(
            "task `{task_name}` env nodes must use named properties like `env KEY=\"value\"`"
        )));
    }

    if node.entries().len() != 1 {
        return Err(RecipeError::new(format!(
            "task `{task_name}` env nodes must define exactly one variable"
        )));
    }

    let entry = &node.entries()[0];
    let name = entry
        .name()
        .ok_or_else(|| RecipeError::new(format!("task `{task_name}` env name is missing")))?;

    Ok(EnvironmentSpec {
        name: name.value().into(),
        value: parse_string_value(entry, "environment value")?,
    })
}

fn parse_artifact(node: &KdlNode, task_name: &str) -> Result<ArtifactSpec, RecipeError> {
    let name = parse_required_string_argument(node, "artifact name")?;
    let path = parse_optional_string_property(node, "path")?.ok_or_else(|| {
        RecipeError::new(format!(
            "task `{task_name}` artifact `{}` is missing a `path` property",
            name.as_str()
        ))
    })?;

    Ok(ArtifactSpec {
        name,
        path: FilePath::from(path),
    })
}

fn validate_tasks(tasks: &[Task]) -> Result<(), RecipeError> {
    let mut names = BTreeSet::new();
    for task in tasks {
        if !names.insert(task.name.as_str().to_owned()) {
            return Err(RecipeError::new(format!(
                "duplicate task name `{}`",
                task.name.as_str()
            )));
        }
    }

    for task in tasks {
        for dependency in &task.dependencies {
            if !names.contains(dependency.as_str()) {
                return Err(RecipeError::new(format!(
                    "task `{}` depends on unknown task `{}`",
                    task.name.as_str(),
                    dependency.as_str()
                )));
            }
        }
    }

    Ok(())
}

fn expect_node_name(node: &KdlNode, expected: &str) -> Result<(), RecipeError> {
    let actual = node.name().value();
    if actual != expected {
        return Err(RecipeError::new(format!(
            "expected node `{expected}`, found `{actual}`"
        )));
    }
    Ok(())
}

fn ensure_no_children(node: &KdlNode, task_name: &str, run_kind: &str) -> Result<(), RecipeError> {
    if node.children().is_some() {
        return Err(RecipeError::new(format!(
            "task `{task_name}` {run_kind} run does not support child nodes"
        )));
    }
    Ok(())
}

fn parse_required_string_argument(
    node: &KdlNode,
    description: &str,
) -> Result<SharedString, RecipeError> {
    let mut positional = node.entries().iter().filter(|entry| entry.name().is_none());
    let value = positional
        .next()
        .ok_or_else(|| RecipeError::new(format!("missing {description}")))?;

    if positional.next().is_some() {
        return Err(RecipeError::new(format!(
            "{description} must be a single string argument"
        )));
    }

    parse_string_value(value, description)
}

fn parse_optional_string_property(
    node: &KdlNode,
    property_name: &str,
) -> Result<Option<SharedString>, RecipeError> {
    let mut matching = node
        .entries()
        .iter()
        .filter(|entry| entry.name().map(|name| name.value()) == Some(property_name));

    let Some(entry) = matching.next() else {
        return Ok(None);
    };

    if matching.next().is_some() {
        return Err(RecipeError::new(format!(
            "property `{property_name}` must not be repeated"
        )));
    }

    Ok(Some(parse_string_value(entry, property_name)?))
}

fn parse_string_value(entry: &KdlEntry, description: &str) -> Result<SharedString, RecipeError> {
    match entry.value() {
        KdlValue::String(value) => Ok(value.as_str().into()),
        _ => Err(RecipeError::new(format!(
            "{description} must be a string value"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use crate::parse_recipe::parse_recipe;
    use crate::run_spec::RunSpec;
    use expect_test::expect;
    use nao_base::shared_string::SharedString;

    #[test]
    fn parses_documented_recipe_shape() {
        let recipe = parse_recipe(
            r#"
            recipe "default" {
              task "build" {
                run shell="cargo build --workspace"
                artifact "workspace-target" path="target"
              }

              task "lint" {
                run shell="cargo clippy --workspace --all-targets --all-features -- -D warnings"
              }

              task "test" {
                depends-on "build"
                run shell="cargo nextest run --workspace --all-targets --all-features"
              }

              task "verify-docs" {
                run script="./scripts/check-docs.sh"
                env RUST_LOG="warn"
              }

              task "image" {
                depends-on "build"
                run container="ghcr.io/example/packager:latest" {
                  args "--input" "target" "--output" "dist/image.tar"
                }
                artifact "container-image" path="dist/image.tar"
              }
            }
            "#,
        )
        .unwrap();

        assert_eq!(recipe.name, SharedString::from("default"));
        assert_eq!(recipe.tasks.len(), 5);
        assert_eq!(recipe.tasks[2].dependencies[0].as_str(), "build");
        assert_eq!(
            recipe.tasks[3].environment[0].name,
            SharedString::from("RUST_LOG")
        );

        let RunSpec::Container(container) = &recipe.tasks[4].run else {
            panic!("expected container run");
        };
        assert_eq!(container.args.len(), 4);
        assert_eq!(recipe.tasks[4].artifacts[0].path.as_str(), "dist/image.tar");
    }

    #[test]
    fn rejects_duplicate_task_names() {
        let error = parse_recipe(
            r#"
            recipe "default" {
              task "build" {
                run shell="cargo build"
              }

              task "build" {
                run shell="cargo test"
              }
            }
            "#,
        )
        .unwrap_err();

        expect!["duplicate task name `build`"].assert_eq(&error.to_string());
    }

    #[test]
    fn rejects_unknown_dependencies() {
        let error = parse_recipe(
            r#"
            recipe "default" {
              task "test" {
                depends-on "build"
                run shell="cargo test"
              }
            }
            "#,
        )
        .unwrap_err();

        expect!["task `test` depends on unknown task `build`"].assert_eq(&error.to_string());
    }

    #[test]
    fn rejects_invalid_run_configuration() {
        let error = parse_recipe(
            r#"
            recipe "default" {
              task "build" {
                run shell="cargo build" script="./build.sh"
              }
            }
            "#,
        )
        .unwrap_err();

        expect![
            "task `build` run node must define exactly one of `shell`, `script`, or `container`"
        ]
        .assert_eq(&error.to_string());
    }

    #[test]
    fn rejects_invalid_env_configuration() {
        let error = parse_recipe(
            r#"
            recipe "default" {
              task "build" {
                run shell="cargo build"
                env "RUST_LOG" "warn"
              }
            }
            "#,
        )
        .unwrap_err();

        expect!["task `build` env nodes must use named properties like `env KEY=\"value\"`"]
            .assert_eq(&error.to_string());
    }
}
