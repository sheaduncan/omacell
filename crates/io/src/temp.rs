//! Private, automatically cleaned temporary directories for format bridges.

use std::fs::DirBuilder;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::DirBuilderExt;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) struct PrivateTempDir {
    path: PathBuf,
}

impl PrivateTempDir {
    pub(crate) fn new(tag: &str) -> std::io::Result<Self> {
        let root = std::env::temp_dir()
            .canonicalize()
            .map_err(|err| std::io::Error::new(err.kind(), format!("resolve temp dir: {err}")))?;
        loop {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = root.join(format!("omacell-{tag}-{}-{sequence}", std::process::id()));
            let mut builder = DirBuilder::new();
            #[cfg(unix)]
            builder.mode(0o700);
            match builder.create(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err),
            }
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PrivateTempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
