use nao_base::err;
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_base::shared_string::SharedString;
use nao_pal::process_command::ProcessCommand;
use nao_recipe::{RunSpec, Task};

pub(super) fn build_process_command(
    recipe_directory: &FilePath,
    task: &Task,
) -> NaoResult<ProcessCommand> {
    let environment = task
        .environment
        .iter()
        .map(
            |variable| nao_pal::process_environment_variable::ProcessEnvironmentVariable {
                name: variable.name.clone(),
                value: variable.value.clone(),
            },
        )
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
                working_directory: Some(recipe_directory.clone()),
                environment,
            })
        }
        RunSpec::Script(script) => Ok(ProcessCommand {
            executable: SharedString::from(script.as_str()),
            arguments: Vec::new(),
            working_directory: Some(recipe_directory.clone()),
            environment,
        }),
        RunSpec::Container(container) => Err(err!(
            "container execution is not implemented yet for task `{}` with image `{}`",
            task.name.as_str(),
            container.image.as_str()
        )),
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
