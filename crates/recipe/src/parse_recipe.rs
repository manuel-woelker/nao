use crate::artifact_spec::ArtifactSpec;
use crate::dependency_name::DependencyName;
use crate::environment_spec::EnvironmentSpec;
use crate::live_display::LiveDisplay;
use crate::recipe::Recipe;
use crate::recipe_config::RecipeConfig;
use crate::recipe_error::RecipeError;
use crate::run_spec::{ContainerRunSpec, RunSpec};
use crate::task::Task;
use crate::task_name::TaskName;
use kdl::{KdlDiagnostic, KdlDocument, KdlEntry, KdlError, KdlNode, KdlValue};
use miette::{GraphicalReportHandler, GraphicalTheme, SourceSpan};
use nao_base::error::NaoError;
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_base::result::ResultExt;
use nao_base::shared_string::SharedString;
use nao_pal::pal::Pal;
use nao_pal::pal_real::PalReal;
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{Display, Formatter};

/// Parses a recipe from a KDL source string.
pub fn parse_recipe(source: &str) -> NaoResult<Recipe> {
    let document: KdlDocument = source
        .parse::<KdlDocument>()
        .map_err(render_kdl_parse_error)
        .with_context(|| "failed to parse recipe KDL")?;
    parse_recipe_document(source, &document).map_err(|error| NaoError::std(error))
}

/// Loads and parses a recipe file using the real platform implementation.
pub fn load_recipe(path: &FilePath) -> NaoResult<Recipe> {
    let pal = PalReal::new_handle();
    load_recipe_with_pal(&*pal, path)
}

/// Loads and parses a recipe file using the supplied platform abstraction.
pub fn load_recipe_with_pal(pal: &dyn Pal, path: &FilePath) -> NaoResult<Recipe> {
    let source = pal.read_file_to_string(path)?;
    let mut recipe = parse_recipe(&source)?;
    if recipe.config.max_parallel_tasks.is_none() {
        recipe.config.max_parallel_tasks = Some(pal.default_parallelism());
    }
    Ok(recipe)
}

#[derive(Debug)]
struct KdlParseDiagnosticError {
    message: SharedString,
}

impl Display for KdlParseDiagnosticError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message.as_str())
    }
}

impl Error for KdlParseDiagnosticError {}

fn render_kdl_parse_error(error: KdlError) -> KdlParseDiagnosticError {
    let mut rendered = String::new();
    let handler = GraphicalReportHandler::new_themed(GraphicalTheme::unicode_nocolor())
        .with_links(false)
        .without_cause_chain();

    let render_result = if let Some(diagnostic) = error.diagnostics.first() {
        handler.render_report(&mut rendered, diagnostic)
    } else {
        handler.render_report(&mut rendered, &error)
    };

    if render_result.is_err() {
        rendered.push_str("Failed to parse KDL document");
        if let Some(diagnostic) = error.diagnostics.first() {
            rendered.push_str("\n- ");
            rendered.push_str(&render_kdl_diagnostic(diagnostic));
        }
    }

    KdlParseDiagnosticError {
        message: SharedString::from(rendered),
    }
}

fn render_kdl_diagnostic(diagnostic: &KdlDiagnostic) -> String {
    let mut rendered = diagnostic.to_string();

    if let Some(help) = &diagnostic.help {
        rendered.push_str(" (help: ");
        rendered.push_str(help);
        rendered.push(')');
    }

    rendered
}

fn parse_recipe_document(source: &str, document: &KdlDocument) -> Result<Recipe, RecipeError> {
    let nodes = document.nodes();
    if nodes.len() != 1 {
        return Err(RecipeError::with_rendered_location(
            format!(
                "expected exactly one top-level recipe node, found {}",
                nodes.len()
            ),
            render_source_span(source, document.span()),
        ));
    }

    let recipe_node = &nodes[0];
    expect_node_name(source, recipe_node, "recipe")?;
    let recipe_name = parse_required_string_argument(source, recipe_node, "recipe name")?;
    let children = recipe_node.children().ok_or_else(|| {
        recipe_error_for_node(
            source,
            recipe_node,
            "recipe node must contain task definitions",
        )
    })?;

    let mut config = RecipeConfig::default();
    let mut config_node = None;
    let mut tasks = Vec::new();
    for child in children.nodes() {
        match child.name().value() {
            "config" => {
                if config_node.is_some() {
                    return Err(recipe_error_for_node(
                        source,
                        child,
                        "recipe cannot define multiple config nodes",
                    ));
                }
                config = parse_recipe_config(source, child)?;
                config_node = Some(child);
            }
            "task" => tasks.push(parse_task(source, child)?),
            other => {
                return Err(recipe_error_for_node(
                    source,
                    child,
                    format!("recipe contains unsupported node `{other}`"),
                ));
            }
        }
    }

    let task_nodes = children
        .nodes()
        .iter()
        .filter(|node| node.name().value() == "task")
        .cloned()
        .collect::<Vec<_>>();
    validate_tasks(source, &tasks, &task_nodes)?;

    Ok(Recipe {
        name: recipe_name,
        config,
        tasks,
    })
}

fn parse_recipe_config(source: &str, node: &KdlNode) -> Result<RecipeConfig, RecipeError> {
    if node.children().is_some() {
        return Err(recipe_error_for_node(
            source,
            node,
            "config node does not support child nodes",
        ));
    }

    for entry in node.entries() {
        let Some(name) = entry.name() else {
            return Err(recipe_error_for_node(
                source,
                node,
                "config nodes must use named properties",
            ));
        };

        if name.value() != "live-display" && name.value() != "max-parallel-tasks" {
            return Err(recipe_error_for_entry(
                source,
                entry,
                format!("config property `{}` is not supported", name.value()),
            ));
        }
    }

    let mut config = RecipeConfig::default();
    if let Some(live_display) = parse_optional_string_property(source, node, "live-display")? {
        config.live_display = LiveDisplay::parse(live_display.as_str()).ok_or_else(|| {
            recipe_error_for_node(
                source,
                node,
                format!(
                    "config live-display must be one of `single-line` or `line-per-task`, found `{}`",
                    live_display.as_str()
                ),
            )
        })?;
    }
    if let Some(max_parallel_tasks) =
        parse_optional_usize_property(source, node, "max-parallel-tasks")?
    {
        if max_parallel_tasks == 0 {
            return Err(recipe_error_for_node(
                source,
                node,
                "config max-parallel-tasks must be at least 1",
            ));
        }
        config.max_parallel_tasks = Some(max_parallel_tasks);
    }

    Ok(config)
}

fn parse_task(source: &str, node: &KdlNode) -> Result<Task, RecipeError> {
    let name = TaskName(parse_required_string_argument(source, node, "task name")?);
    let description = parse_optional_string_property(source, node, "description")?;
    let children = node.children().ok_or_else(|| {
        recipe_error_for_node(
            source,
            node,
            format!("task `{}` must have child nodes", name.as_str()),
        )
    })?;

    let mut dependencies = Vec::new();
    let mut run = None;
    let mut environment = Vec::new();
    let mut artifacts = Vec::new();

    for child in children.nodes() {
        match child.name().value() {
            "depends-on" => {
                dependencies.push(DependencyName(parse_required_string_argument(
                    source,
                    child,
                    "dependency name",
                )?));
            }
            "run" => {
                if run.is_some() {
                    return Err(recipe_error_for_node(
                        source,
                        child,
                        format!("task `{}` cannot define multiple run nodes", name.as_str()),
                    ));
                }
                run = Some(parse_run(source, child, name.as_str())?);
            }
            "env" => {
                environment.push(parse_environment(source, child, name.as_str())?);
            }
            "artifact" => {
                artifacts.push(parse_artifact(source, child, name.as_str())?);
            }
            other => {
                return Err(recipe_error_for_node(
                    source,
                    child,
                    format!(
                        "task `{}` contains unsupported node `{other}`",
                        name.as_str()
                    ),
                ));
            }
        }
    }

    let run = run.ok_or_else(|| {
        recipe_error_for_node(
            source,
            node,
            format!("task `{}` is missing a run node", name.as_str()),
        )
    })?;

    Ok(Task {
        name,
        description,
        dependencies,
        run,
        environment,
        artifacts,
    })
}

fn parse_run(source: &str, node: &KdlNode, task_name: &str) -> Result<RunSpec, RecipeError> {
    let shell = parse_optional_string_property(source, node, "shell")?;
    let script = parse_optional_string_property(source, node, "script")?;
    let container = parse_optional_string_property(source, node, "container")?;

    let defined = [shell.is_some(), script.is_some(), container.is_some()]
        .into_iter()
        .filter(|present| *present)
        .count();

    if defined != 1 {
        return Err(recipe_error_for_node(
            source,
            node,
            format!(
                "task `{task_name}` run node must define exactly one of `shell`, `script`, or `container`"
            ),
        ));
    }

    if let Some(shell) = shell {
        ensure_no_children(source, node, task_name, "shell")?;
        return Ok(RunSpec::Shell(shell));
    }

    if let Some(script) = script {
        ensure_no_children(source, node, task_name, "script")?;
        return Ok(RunSpec::Script(FilePath::from(script)));
    }

    let args = parse_container_args(source, node, task_name)?;
    Ok(RunSpec::Container(ContainerRunSpec {
        image: container.expect("container property checked above"),
        args,
    }))
}

fn parse_container_args(
    source: &str,
    node: &KdlNode,
    task_name: &str,
) -> Result<Vec<SharedString>, RecipeError> {
    let Some(children) = node.children() else {
        return Ok(Vec::new());
    };

    let mut args = Vec::new();
    for child in children.nodes() {
        if child.name().value() != "args" {
            return Err(recipe_error_for_node(
                source,
                child,
                format!("task `{task_name}` container run only supports `args` child nodes"),
            ));
        }

        for entry in child.entries() {
            if entry.name().is_some() {
                return Err(recipe_error_for_entry(
                    source,
                    entry,
                    format!("task `{task_name}` container args must use positional string values"),
                ));
            }
            args.push(parse_string_value(source, entry, "container argument")?);
        }
    }

    Ok(args)
}

fn parse_environment(
    source: &str,
    node: &KdlNode,
    task_name: &str,
) -> Result<EnvironmentSpec, RecipeError> {
    if !node.entries().iter().all(|entry| entry.name().is_some()) {
        return Err(recipe_error_for_node(
            source,
            node,
            format!(
                "task `{task_name}` env nodes must use named properties like `env KEY=\"value\"`"
            ),
        ));
    }

    if node.entries().len() != 1 {
        return Err(recipe_error_for_node(
            source,
            node,
            format!("task `{task_name}` env nodes must define exactly one variable"),
        ));
    }

    let entry = &node.entries()[0];
    let name = entry.name().ok_or_else(|| {
        recipe_error_for_node(
            source,
            node,
            format!("task `{task_name}` env name is missing"),
        )
    })?;

    Ok(EnvironmentSpec {
        name: name.value().into(),
        value: parse_string_value(source, entry, "environment value")?,
    })
}

fn parse_artifact(
    source: &str,
    node: &KdlNode,
    task_name: &str,
) -> Result<ArtifactSpec, RecipeError> {
    let name = parse_required_string_argument(source, node, "artifact name")?;
    let path = parse_optional_string_property(source, node, "path")?.ok_or_else(|| {
        recipe_error_for_node(
            source,
            node,
            format!(
                "task `{task_name}` artifact `{}` is missing a `path` property",
                name.as_str()
            ),
        )
    })?;

    Ok(ArtifactSpec {
        name,
        path: FilePath::from(path),
    })
}

fn validate_tasks(source: &str, tasks: &[Task], task_nodes: &[KdlNode]) -> Result<(), RecipeError> {
    let mut names = BTreeSet::new();
    for (task, task_node) in tasks.iter().zip(task_nodes) {
        if task.name.as_str().contains('_') {
            return Err(recipe_error_for_node(
                source,
                task_node,
                format!(
                    "task name `{}` cannot contain `_` because `_` is reserved for wildcard task selectors",
                    task.name.as_str()
                ),
            ));
        }
        if !names.insert(task.name.as_str().to_owned()) {
            return Err(recipe_error_for_node(
                source,
                task_node,
                format!("duplicate task name `{}`", task.name.as_str()),
            ));
        }
    }

    for (task, task_node) in tasks.iter().zip(task_nodes) {
        for dependency in &task.dependencies {
            if !names.contains(dependency.as_str()) {
                return Err(recipe_error_for_node(
                    source,
                    task_node,
                    format!(
                        "task `{}` depends on unknown task `{}`",
                        task.name.as_str(),
                        dependency.as_str()
                    ),
                ));
            }
        }
    }

    Ok(())
}

fn expect_node_name(source: &str, node: &KdlNode, expected: &str) -> Result<(), RecipeError> {
    let actual = node.name().value();
    if actual != expected {
        return Err(recipe_error_for_node(
            source,
            node,
            format!("expected node `{expected}`, found `{actual}`"),
        ));
    }
    Ok(())
}

fn ensure_no_children(
    source: &str,
    node: &KdlNode,
    task_name: &str,
    run_kind: &str,
) -> Result<(), RecipeError> {
    if node.children().is_some() {
        return Err(recipe_error_for_node(
            source,
            node,
            format!("task `{task_name}` {run_kind} run does not support child nodes"),
        ));
    }
    Ok(())
}

fn parse_required_string_argument(
    source: &str,
    node: &KdlNode,
    description: &str,
) -> Result<SharedString, RecipeError> {
    let mut positional = node.entries().iter().filter(|entry| entry.name().is_none());
    let value = positional
        .next()
        .ok_or_else(|| recipe_error_for_node(source, node, format!("missing {description}")))?;

    if positional.next().is_some() {
        return Err(recipe_error_for_node(
            source,
            node,
            format!("{description} must be a single string argument"),
        ));
    }

    parse_string_value(source, value, description)
}

fn parse_optional_string_property(
    source: &str,
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
        return Err(recipe_error_for_entry(
            source,
            entry,
            format!("property `{property_name}` must not be repeated"),
        ));
    }

    Ok(Some(parse_string_value(source, entry, property_name)?))
}

fn parse_optional_usize_property(
    source: &str,
    node: &KdlNode,
    property_name: &str,
) -> Result<Option<usize>, RecipeError> {
    let mut matching = node
        .entries()
        .iter()
        .filter(|entry| entry.name().map(|name| name.value()) == Some(property_name));

    let Some(entry) = matching.next() else {
        return Ok(None);
    };

    if matching.next().is_some() {
        return Err(recipe_error_for_entry(
            source,
            entry,
            format!("property `{property_name}` must not be repeated"),
        ));
    }

    match entry.value() {
        KdlValue::Integer(value) => usize::try_from(*value).map(Some).map_err(|_| {
            recipe_error_for_entry(
                source,
                entry,
                format!("property `{property_name}` must be a non-negative integer value"),
            )
        }),
        _ => Err(recipe_error_for_entry(
            source,
            entry,
            format!("property `{property_name}` must be a non-negative integer value"),
        )),
    }
}

fn parse_string_value(
    source: &str,
    entry: &KdlEntry,
    description: &str,
) -> Result<SharedString, RecipeError> {
    match entry.value() {
        KdlValue::String(value) => Ok(value.as_str().into()),
        _ => {
            let message = format!("{description} must be a string value");
            if source.is_empty() {
                Err(RecipeError::new(message))
            } else {
                Err(recipe_error_for_entry(source, entry, message))
            }
        }
    }
}

fn recipe_error_for_node(
    source: &str,
    node: &KdlNode,
    message: impl Into<SharedString>,
) -> RecipeError {
    RecipeError::with_rendered_location(message, render_source_span(source, node.span()))
}

fn recipe_error_for_entry(
    source: &str,
    entry: &KdlEntry,
    message: impl Into<SharedString>,
) -> RecipeError {
    RecipeError::with_rendered_location(message, render_source_span(source, entry.span()))
}

fn render_source_span(source: &str, span: SourceSpan) -> String {
    let offset = span.offset().min(source.len());
    let line_number = source[..offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1;
    let line_start = source[..offset]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let line_end = source[offset..]
        .find('\n')
        .map(|index| offset + index)
        .unwrap_or(source.len());
    let line = &source[line_start..line_end];
    let column = source[line_start..offset].chars().count() + 1;
    let marker_len = if span.is_empty() {
        1
    } else {
        let span_end = (offset + span.len()).min(source.len());
        source[offset..span_end].chars().count().max(1)
    };
    let gutter_width = line_number.to_string().len();
    let pointer_padding = " ".repeat(column.saturating_sub(1));
    let marker = "^".repeat(marker_len);

    format!(
        "  --> line {line_number}, column {column}\n{line_number:>gutter_width$} | {line}\n{} | {pointer_padding}{marker}",
        " ".repeat(gutter_width),
    )
}

#[cfg(test)]
mod tests {
    use crate::parse_recipe::{load_recipe_with_pal, parse_recipe, render_kdl_parse_error};
    use crate::run_spec::RunSpec;
    use crate::{LiveDisplay, RecipeConfig};
    use expect_test::expect;
    use kdl::{KdlDiagnostic, KdlError};
    use miette::{Severity, SourceSpan};
    use nao_base::file_path::FilePath;
    use nao_base::shared_string::SharedString;
    use nao_pal::pal_mock::PalMock;
    use std::sync::Arc;

    #[test]
    fn parses_documented_recipe_shape() {
        let recipe = parse_recipe(
            r#"
            recipe "default" {
              task "build" description="Build the workspace" {
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
        assert_eq!(recipe.config, RecipeConfig::default());
        assert_eq!(recipe.tasks.len(), 5);
        assert_eq!(
            recipe.tasks[0].description,
            Some(SharedString::from("Build the workspace"))
        );
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

        assert!(
            error
                .to_test_string()
                .contains("duplicate task name `build`")
        );
    }

    #[test]
    fn rejects_task_names_with_underscores() {
        let error = parse_recipe(
            r#"
            recipe "default" {
              task "unit_tests" {
                run shell="cargo test"
              }
            }
            "#,
        )
        .unwrap_err();

        assert!(error.to_test_string().contains(
            "task name `unit_tests` cannot contain `_` because `_` is reserved for wildcard task selectors"
        ));
    }

    #[test]
    fn defaults_recipe_live_display_to_line_per_task() {
        let recipe = parse_recipe(
            r#"
            recipe "default" {
              task "build" {
                run shell="cargo build"
              }
            }
            "#,
        )
        .unwrap();

        assert_eq!(recipe.config.live_display, LiveDisplay::LinePerTask);
    }

    #[test]
    fn parses_recipe_live_display_config() {
        let recipe = parse_recipe(
            r#"
            recipe "default" {
              config live-display="single-line"

              task "build" {
                run shell="cargo build"
              }
            }
            "#,
        )
        .unwrap();

        assert_eq!(recipe.config.live_display, LiveDisplay::SingleLine);
    }

    #[test]
    fn defaults_recipe_max_parallel_tasks_to_one() {
        let recipe = parse_recipe(
            r#"
            recipe "default" {
              task "build" {
                run shell="cargo build"
              }
            }
            "#,
        )
        .unwrap();

        assert_eq!(recipe.config.max_parallel_tasks, None);
    }

    #[test]
    fn parses_recipe_max_parallel_tasks_config() {
        let recipe = parse_recipe(
            r#"
            recipe "default" {
              config max-parallel-tasks=4

              task "build" {
                run shell="cargo build"
              }
            }
            "#,
        )
        .unwrap();

        assert_eq!(recipe.config.max_parallel_tasks, Some(4));
    }

    #[test]
    fn rejects_zero_recipe_max_parallel_tasks_config() {
        let error = parse_recipe(
            r#"
            recipe "default" {
              config max-parallel-tasks=0

              task "build" {
                run shell="cargo build"
              }
            }
            "#,
        )
        .unwrap_err();

        assert!(
            error
                .to_test_string()
                .contains("config max-parallel-tasks must be at least 1")
        );
    }

    #[test]
    fn rejects_invalid_recipe_max_parallel_tasks_config_type() {
        let error = parse_recipe(
            r#"
            recipe "default" {
              config max-parallel-tasks="many"

              task "build" {
                run shell="cargo build"
              }
            }
            "#,
        )
        .unwrap_err();

        assert!(
            error
                .to_test_string()
                .contains("property `max-parallel-tasks` must be a non-negative integer value")
        );
    }

    #[test]
    fn rejects_invalid_recipe_live_display_config() {
        let error = parse_recipe(
            r#"
            recipe "default" {
              config live-display="sideways"

              task "build" {
                run shell="cargo build"
              }
            }
            "#,
        )
        .unwrap_err();

        assert!(error.to_test_string().contains(
            "config live-display must be one of `single-line` or `line-per-task`, found `sideways`"
        ));
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

        assert!(
            error
                .to_test_string()
                .contains("task `test` depends on unknown task `build`")
        );
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

        let rendered = error.to_test_string();

        assert!(rendered.contains(
            "task `build` run node must define exactly one of `shell`, `script`, or `container`"
        ));
        assert!(rendered.contains("--> line 4, column 17"));
        assert!(
            rendered
                .contains("4 |                 run shell=\"cargo build\" script=\"./build.sh\"")
        );
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

        assert!(
            error.to_test_string().contains(
                "task `build` env nodes must use named properties like `env KEY=\"value\"`"
            )
        );
    }

    #[test]
    fn adds_context_for_invalid_kdl() {
        let error = parse_recipe("recipe \"default\" {").unwrap_err();
        let rendered = error.to_test_string();

        assert!(rendered.contains("failed to parse recipe KDL"));
        assert!(rendered.contains("No closing '}' for child block"));
        assert!(rendered.contains("recipe \"default\" {"));
        assert!(rendered.contains("not closed"));
    }

    #[test]
    fn only_renders_first_kdl_diagnostic() {
        let error = KdlError {
            input: Arc::new("broken".to_owned()),
            diagnostics: vec![
                KdlDiagnostic {
                    input: Arc::new("broken".to_owned()),
                    span: SourceSpan::from((0, 3)),
                    message: Some("first diagnostic".to_owned()),
                    label: Some("first".to_owned()),
                    help: None,
                    severity: Severity::Error,
                },
                KdlDiagnostic {
                    input: Arc::new("broken".to_owned()),
                    span: SourceSpan::from((3, 3)),
                    message: Some("second diagnostic".to_owned()),
                    label: Some("second".to_owned()),
                    help: None,
                    severity: Severity::Error,
                },
            ],
        };

        let rendered = render_kdl_parse_error(error).to_string();

        assert!(rendered.contains("first diagnostic"));
        assert!(!rendered.contains("second diagnostic"));
    }

    #[test]
    fn loads_recipe_via_pal_mock() {
        let pal = PalMock::new();
        pal.set_default_parallelism(8);
        pal.set_file(
            "nao.kdl",
            r#"
            recipe "default" {
              task "build" description="Build the workspace" {
                run shell="cargo build"
              }
            }
            "#,
        );

        let recipe = load_recipe_with_pal(&pal, &FilePath::from("nao.kdl")).unwrap();

        assert_eq!(recipe.name, SharedString::from("default"));
        assert_eq!(recipe.tasks.len(), 1);
        assert_eq!(recipe.config.max_parallel_tasks, Some(8));
        expect!["READ FILE: nao.kdl\n"].assert_eq(&pal.get_effects());
    }
}
