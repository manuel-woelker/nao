use nao_base::cli::try_main_with_headline;
use nao_base::err;
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_pal::pal_real::PalReal;
use nao_recipe::{RunSpec, Task, load_recipe_with_pal};
use std::path::PathBuf;
use std::process::ExitCode;

xflags::xflags! {
    cmd nao {
        optional --list
        repeated task_name: String
        optional recipe_file: PathBuf
    }
}

fn main() -> ExitCode {
    try_main_with_headline("nao CLI failed", run)
}

fn run() -> NaoResult<()> {
    let flags = Nao::from_env().map_err(|error| err!("{error}"))?;
    let recipe_path = flags
        .recipe_file
        .unwrap_or_else(|| PathBuf::from("nao.kdl"));
    let pal = PalReal::new_handle();
    let recipe = load_recipe_with_pal(&*pal, &FilePath::new(&recipe_path))?;

    if flags.list {
        print_task_list(&recipe.tasks);
        return Ok(());
    }

    if flags.task_name.is_empty() {
        return Err(err!("usage: nao [--list] [task-name...] [recipe-file]"));
    }

    for (index, task_name) in flags.task_name.iter().enumerate() {
        let task = recipe
            .tasks
            .iter()
            .find(|task| task.name.as_str() == task_name)
            .ok_or_else(|| err!("task `{task_name}` not found"))?;

        if index > 0 {
            println!();
        }
        print_task_preview(task);
    }

    Ok(())
}

fn print_task_list(tasks: &[Task]) {
    let width = tasks
        .iter()
        .map(|task| task.name.as_str().len())
        .max()
        .unwrap_or(0);

    println!("Available tasks:");
    println!();

    for task in tasks {
        let bold_name = format!("\u{1b}[1m{:<width$}\u{1b}[0m", task.name.as_str());
        match &task.description {
            Some(description) => println!("  {bold_name}  {description}"),
            None => println!("  {bold_name}"),
        }
    }
}

fn print_task_preview(task: &Task) {
    println!("Pretending to run task:");
    println!();
    println!("  name: {}", task.name.as_str());

    match &task.description {
        Some(description) => println!("  description: {description}"),
        None => println!("  description: <none>"),
    }

    if task.dependencies.is_empty() {
        println!("  dependencies: <none>");
    } else {
        let dependencies = task
            .dependencies
            .iter()
            .map(|dependency| dependency.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        println!("  dependencies: {dependencies}");
    }

    match &task.run {
        RunSpec::Shell(command) => {
            println!("  run: shell");
            println!("  command: {command}");
        }
        RunSpec::Script(script) => {
            println!("  run: script");
            println!("  path: {}", script.as_str());
        }
        RunSpec::Container(container) => {
            println!("  run: container");
            println!("  image: {}", container.image);
            if container.args.is_empty() {
                println!("  args: <none>");
            } else {
                println!("  args: {}", container.args.join(" "));
            }
        }
    }

    if task.environment.is_empty() {
        println!("  env: <none>");
    } else {
        println!("  env:");
        for environment in &task.environment {
            println!("    {}={}", environment.name, environment.value);
        }
    }

    if task.artifacts.is_empty() {
        println!("  artifacts: <none>");
    } else {
        println!("  artifacts:");
        for artifact in &task.artifacts {
            println!("    {} -> {}", artifact.name, artifact.path);
        }
    }
}
