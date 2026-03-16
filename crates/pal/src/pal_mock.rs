use crate::pal::{FileChangeCallback, Pal, ReadSeek};
use expect_test::Expect;
use nao_base::RwLock;
use nao_base::file_path::FilePath;
use nao_base::result::{NaoResult, OptionExt};
use nao_base::timestamp::Timestamp;
use std::collections::HashMap;
use std::fmt::Debug;
use std::io::{Cursor, Write};
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct PalMock {
    inner: Arc<RwLock<PalMockInner>>,
}

#[derive(Default)]
struct PalMockInner {
    effects_string: String,
    file_map: HashMap<FilePath, Vec<u8>>,
    current_timestamp: Timestamp,
}

impl PalMock {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(PalMockInner {
                effects_string: String::new(),
                file_map: HashMap::new(),
                current_timestamp: Timestamp::new(0),
            })),
        }
    }

    pub fn log_effect(&self, effect: impl AsRef<str>) {
        let mut inner = self.inner.write();
        inner.effects_string.push_str(effect.as_ref());
        inner.effects_string.push('\n');
    }

    pub fn verify_effects(&self, expected: Expect) {
        expected.assert_eq(&self.inner.read().effects_string);
        self.inner.write().effects_string.clear();
    }

    #[allow(dead_code)]
    pub fn get_effects(&self) -> String {
        self.inner.read().effects_string.clone()
    }

    pub fn clear_effects(&self) {
        self.inner.write().effects_string.clear();
    }

    pub fn set_file(&self, file_path: &str, content: impl Into<Vec<u8>>) {
        self.inner
            .write()
            .file_map
            .insert(FilePath::from(file_path), content.into());
    }
}

impl Pal for PalMock {
    fn file_exists(&self, _path: &FilePath) -> NaoResult<bool> {
        Ok(false)
    }

    fn read_file(&self, path: &FilePath) -> NaoResult<Box<dyn ReadSeek + 'static>> {
        self.log_effect(format!("READ FILE: {path}"));
        Ok(Box::new(Cursor::new(
            self.inner
                .read()
                .file_map
                .get(path)
                .with_context(|| format!("File '{path}' does not exist"))?
                .clone(),
        )))
    }

    fn walk_directory(
        &self,
        path: &FilePath,
        _globs: &[String],
    ) -> NaoResult<Box<dyn Iterator<Item = NaoResult<FilePath>> + '_>> {
        let mut result = vec![];
        for file_path in self.inner.read().file_map.keys() {
            if file_path.as_path().starts_with(path.as_path()) {
                result.push(Ok(file_path.clone()))
            }
        }
        Ok(Box::new(result.into_iter()))
    }

    fn watch_directory(
        &self,
        _directory: &FilePath,
        _globs: &[String],
        _callback: FileChangeCallback,
    ) -> NaoResult<()> {
        Ok(())
    }

    fn now(&self) -> Timestamp {
        self.inner.read().current_timestamp
    }
}

impl Debug for PalMock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PalMock").finish()
    }
}

pub struct MockFile {
    path: FilePath,
    data: Vec<u8>,
    pal_mock: PalMock,
}

impl MockFile {
    pub fn new(path: &FilePath, pal_mock: PalMock) -> Self {
        Self {
            path: path.clone(),
            data: vec![],
            pal_mock,
        }
    }
}

impl Write for MockFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.data.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for MockFile {
    fn drop(&mut self) {
        self.pal_mock.log_effect(format!(
            "WRITE FILE: {} -> {}",
            self.path,
            String::from_utf8_lossy(&self.data)
        ));
        self.pal_mock
            .inner
            .write()
            .file_map
            .insert(self.path.clone(), self.data.clone());
    }
}
