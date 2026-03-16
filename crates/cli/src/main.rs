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
        for task in &recipe.tasks {
            match &task.description {
                Some(description) => println!("{}: {}", task.name.as_str(), description),
                None => println!("{}", task.name.as_str()),
            }
        }
        Ok(())
    } else {
        Err(err!("usage: nao --list [recipe-file]"))
    }
}
