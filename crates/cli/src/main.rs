use nao_base::cli::try_main_with_headline;
use nao_base::err;
use nao_base::result::NaoResult;
use nao_recipe::load_recipe;
use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    try_main_with_headline("nao CLI failed", run)
}

fn run() -> NaoResult<()> {
    let recipe_path = env::args()
        .nth(1)
        .ok_or_else(|| err!("usage: nao <recipe-file>"))?;
    let recipe = load_recipe(&recipe_path)?;

    println!(
        "loaded recipe `{}` with {} task(s)",
        recipe.name,
        recipe.tasks.len()
    );

    Ok(())
}
