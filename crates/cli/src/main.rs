use nao_base::cli::try_main_with_headline;
use nao_base::err;
use nao_base::result::NaoResult;
use nao_recipe::load_recipe;
use std::path::PathBuf;
use std::process::ExitCode;

xflags::xflags! {
    cmd nao {
        optional --list
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
    let recipe = load_recipe(&recipe_path)?;

    if flags.list {
        let width = recipe
            .tasks
            .iter()
            .map(|task| task.name.as_str().len())
            .max()
            .unwrap_or(0);

        println!("Available tasks:");
        println!();

        for task in &recipe.tasks {
            let bold_name = format!("\u{1b}[1m{:<width$}\u{1b}[0m", task.name.as_str());
            match &task.description {
                Some(description) => println!("  {bold_name}  {description}"),
                None => println!("  {bold_name}"),
            }
        }
        Ok(())
    } else {
        Err(err!("usage: nao --list [recipe-file]"))
    }
}
