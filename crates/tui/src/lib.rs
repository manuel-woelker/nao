use crate::app::App;
use nao_base::file_path::FilePath;
use nao_base::result::NaoResult;
use nao_pal::pal::PalHandle;

pub mod app;
pub mod artifact_store;

/// How should the main CLI launch the full-screen TUI?
///
/// It calls this helper so the TUI crate stays library-only while the primary
/// `nao` executable owns argument parsing and top-level process startup.
pub fn run(pal: PalHandle, recipe_path: FilePath) -> NaoResult<()> {
    let mut app = App::new(pal, recipe_path)?;
    app.run()
}
