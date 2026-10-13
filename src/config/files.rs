//! Filesystem access and construction of its production implementation.

use std::{fmt::Debug, path::Path};

pub(crate) trait Files: Debug + Send + Sync {
    fn read(&self, path: &Path) -> std::io::Result<String>;
    fn exists(&self, path: &Path) -> std::io::Result<bool>;
}

#[derive(Debug)]
pub(crate) struct NativeFiles;

impl Files for NativeFiles {
    fn read(&self, path: &Path) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn exists(&self, path: &Path) -> std::io::Result<bool> {
        path.try_exists()
    }
}
