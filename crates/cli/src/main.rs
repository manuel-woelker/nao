mod runner;

use nao_base::cli::try_main_with_headline;
use nao_base::err;
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_pal::pal_real::PalReal;
use runner::Runner;
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
    let runner = Runner::new(PalReal::new_handle());
    let output = runner.execute(&FilePath::new(&recipe_path), flags.list, &flags.task_name)?;

    print!("{output}");
    Ok(())
}
