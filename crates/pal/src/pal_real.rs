use crate::pal::{FileChangeCallback, FileChangeEvent, Pal, PalHandle, ReadSeek};
use ignore::WalkBuilder;
use ignore::gitignore::GitignoreBuilder;
use ignore::overrides::OverrideBuilder;
use nao_base::RwLock;
use nao_base::bail;
use nao_base::file_path::FilePath;
use nao_base::logging::{error, info};
use nao_base::result::{NaoResult, ResultExt};
use nao_base::timestamp::Timestamp;
use notify_debouncer_full::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use std::fmt::Debug;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub struct PalReal {
    base_path: PathBuf,
    watchers: RwLock<Vec<Debouncer<RecommendedWatcher, RecommendedCache>>>,
    reference_instant: Instant,
}

impl PalReal {
    pub fn new_handle() -> PalHandle {
        PalHandle::new(Self::new())
    }

    pub fn new() -> Self {
        let current_dir = std::env::current_dir().expect("Unable to access current directory");
        Self {
            base_path: current_dir,
            watchers: RwLock::new(Vec::new()),
            reference_instant: Instant::now(),
        }
    }

    fn resolve_path(&self, path: &FilePath) -> NaoResult<PathBuf> {
        Ok(self.base_path.join(path.as_path()))
    }

    fn relativize_path(&self, path: &Path) -> NaoResult<FilePath> {
        let relative_path = path.strip_prefix(&self.base_path).with_context(|| {
            format!(
                "Unable to relativize path '{}' against '{}'",
                path.display(),
                self.base_path.display()
            )
        })?;
        Ok(FilePath::new(relative_path))
    }
}

impl Default for PalReal {
    fn default() -> Self {
        Self::new()
    }
}

impl Pal for PalReal {
    fn file_exists(&self, path: &FilePath) -> NaoResult<bool> {
        Ok(std::fs::exists(self.resolve_path(path)?)?)
    }

    fn read_file(&self, path: &FilePath) -> NaoResult<Box<dyn ReadSeek + 'static>> {
        Ok(Box::new(
            File::open(self.resolve_path(path)?)
                .with_context(|| format!("Unable to open file '{}'", path))?,
        ))
    }

    fn walk_directory(
        &self,
        path: &FilePath,
        globs: &[String],
    ) -> NaoResult<Box<dyn Iterator<Item = NaoResult<FilePath>> + '_>> {
        let real_path = self.resolve_path(path)?;
        if !real_path.is_dir() {
            bail!("Path is not a directory: '{}'", path);
        }
        let mut walk_builder = WalkBuilder::new(&real_path);
        let mut overrides = OverrideBuilder::new(&real_path);
        for glob in globs {
            overrides.add(glob)?;
        }
        walk_builder.overrides(overrides.build()?);
        Ok(Box::new(
            walk_builder
                .build()
                .filter(|entry| match entry {
                    Ok(dir_entry) => {
                        if let Some(file_type) = &dir_entry.file_type()
                            && file_type.is_file()
                        {
                            true
                        } else {
                            false
                        }
                    }
                    Err(_) => false,
                })
                .flat_map(|entry| entry.map(|path| self.relativize_path(path.path()))),
        ))
    }

    fn watch_directory(
        &self,
        directory: &FilePath,
        globs: &[String],
        callback: FileChangeCallback,
    ) -> NaoResult<()> {
        let mut gitignore_builder = GitignoreBuilder::new(&self.base_path);
        for glob in globs {
            gitignore_builder.add_line(None, glob)?;
        }
        let gitignore = gitignore_builder.build()?;
        let base_path = self.base_path.clone();
        let mut debouncer = new_debouncer(
            Duration::from_millis(500),
            None,
            move |result: DebounceEventResult| match result {
                Ok(events) => {
                    let mut changed_files = Vec::new();
                    for event in &events {
                        if !(event.kind.is_create()
                            || event.kind.is_modify()
                            || event.kind.is_remove())
                        {
                            continue;
                        }
                        for path in &event.paths {
                            let matches = gitignore.matched_path_or_any_parents(path, false);
                            if !matches.is_ignore()
                                && let Ok(relative_path) = path.strip_prefix(&base_path)
                            {
                                changed_files.push(FilePath::new(relative_path));
                            }
                        }
                    }
                    #[allow(clippy::collapsible_if)]
                    if !changed_files.is_empty() {
                        if let Err(error) = callback(FileChangeEvent { changed_files }) {
                            error!("Failed to call filewatcher callback for {events:?}: {error:?}");
                        }
                    }
                }
                Err(errors) => errors.iter().for_each(|error| println!("{error:?}")),
            },
        )?;
        let path = self.resolve_path(directory)?;
        info!(
            "Watching directory {}, globs: {}",
            directory,
            globs.join(", ")
        );
        debouncer.watch(path, RecursiveMode::Recursive)?;
        self.watchers.write().push(debouncer);
        Ok(())
    }

    fn now(&self) -> Timestamp {
        Timestamp::new(self.reference_instant.elapsed().as_nanos())
    }
}

impl Debug for PalReal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PalReal").finish()
    }
}
