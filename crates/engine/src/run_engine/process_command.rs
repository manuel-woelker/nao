use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_base::shared_string::SharedString;
use nao_pal::process_command::ProcessCommand;
use nao_pal::process_environment_variable::ProcessEnvironmentVariable;
use nao_recipe::{RunSpec, Task};

pub(super) fn build_process_command(
    recipe_path: &FilePath,
    task: &Task,
) -> NaoResult<ProcessCommand> {
    let workspace_directory = recipe_workspace_directory(recipe_path);
    let environment = task
        .environment
        .iter()
        .map(|variable| ProcessEnvironmentVariable {
            name: variable.name.clone(),
            value: variable.value.clone(),
        })
        .collect::<Vec<_>>();

    match &task.run {
        RunSpec::Shell(command) => {
            #[cfg(windows)]
            let (executable, arguments) = (
                SharedString::from("cmd"),
                vec![SharedString::from("/C"), command.clone()],
            );

            #[cfg(not(windows))]
            let (executable, arguments) = (
                SharedString::from("bash"),
                vec![
                    SharedString::from("-o"),
                    SharedString::from("errexit"),
                    SharedString::from("-o"),
                    SharedString::from("nounset"),
                    SharedString::from("-o"),
                    SharedString::from("errtrace"),
                    SharedString::from("-o"),
                    SharedString::from("pipefail"),
                    SharedString::from("-c"),
                    build_bash_shell_script(command.as_str()),
                ],
            );

            Ok(ProcessCommand {
                executable,
                arguments,
                working_directory: Some(workspace_directory),
                environment,
            })
        }
        RunSpec::Script(script) => Ok(ProcessCommand {
            executable: SharedString::from(script.as_str()),
            arguments: Vec::new(),
            working_directory: Some(workspace_directory),
            environment,
        }),
        RunSpec::Container(container) => Ok(ProcessCommand {
            executable: SharedString::from("docker"),
            arguments: build_docker_run_arguments(container, &environment),
            working_directory: Some(workspace_directory),
            environment: Vec::new(),
        }),
    }
}

fn build_docker_run_arguments(
    container: &nao_recipe::ContainerRunSpec,
    environment: &[ProcessEnvironmentVariable],
) -> Vec<SharedString> {
    let mut arguments = vec![
        SharedString::from("run"),
        SharedString::from("--rm"),
        SharedString::from("--volume"),
        SharedString::from(".:/workspace"),
        SharedString::from("--workdir"),
        SharedString::from("/workspace"),
    ];
    for variable in environment {
        arguments.push(SharedString::from("--env"));
        arguments.push(format!("{}={}", variable.name.as_str(), variable.value.as_str()).into());
    }
    arguments.push(container.image.clone());
    arguments.extend(container.args.iter().cloned());
    arguments
}

pub(super) fn recipe_workspace_directory(recipe_path: &FilePath) -> FilePath {
    let recipe_directory = directory_or_current_directory(recipe_path.parent());
    if recipe_path.file_name() == Some("nao.kdl") && recipe_directory.file_name() == Some(".nao") {
        return directory_or_current_directory(recipe_directory.parent());
    }
    recipe_directory
}

fn directory_or_current_directory(path: Option<FilePath>) -> FilePath {
    match path {
        Some(path) if !path.as_str().is_empty() => path,
        _ => FilePath::from("."),
    }
}

#[cfg(not(windows))]
pub(super) fn build_bash_shell_script(command: &str) -> SharedString {
    format!(
        "trap 'rc=$?; printf \"nao: command failed (exit %d) at line %d: %s\\n\" \"$rc\" \"$LINENO\" \"$BASH_COMMAND\" >&2; exit \"$rc\"' ERR\n{command}",
        command = command,
    )
    .into()
}
