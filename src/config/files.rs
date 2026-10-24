//! Filesystem access and construction of its production implementation.

use std::{
    fmt::{Debug, Write as _},
    path::Path,
};

pub(crate) trait RepositoryLock: Debug + Send {}
impl RepositoryLock for std::fs::File {}

pub(crate) trait Files: Debug + Send + Sync {
    fn lock(&self, path: &Path) -> std::io::Result<Box<dyn RepositoryLock>>;
    fn attempt_id(&self) -> std::io::Result<String>;
    fn read(&self, path: &Path) -> std::io::Result<String>;
    fn exists(&self, path: &Path) -> std::io::Result<bool>;
}

#[derive(Debug)]
pub(crate) struct NativeFiles;

impl Files for NativeFiles {
    fn lock(&self, path: &Path) -> std::io::Result<Box<dyn RepositoryLock>> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.try_lock().map_err(std::io::Error::other)?;
        Ok(Box::new(file))
    }
    fn attempt_id(&self) -> std::io::Result<String> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        let hex = bytes.iter().fold(String::new(), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        });
        Ok(format!(
            "{}-{}-{}-{}-{}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..]
        ))
    }

    fn read(&self, path: &Path) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn exists(&self, path: &Path) -> std::io::Result<bool> {
        path.try_exists()
    }
}
