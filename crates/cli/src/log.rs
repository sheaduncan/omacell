//! Tracing to stderr and `~/.local/state/omacell/logs/`.

use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

use omacell_conf::Paths;

const MAX_LOG_BYTES: u64 = 1024 * 1024;

/// Install stderr logging and, when enabled, rotating file logging.
pub fn init(paths: &Paths, verbose: u8, quiet: bool, write_file: bool) {
    let level = if quiet {
        "warn"
    } else {
        match verbose {
            0 => "info",
            1 => "debug",
            _ => "trace",
        }
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    let stderr = tracing_subscriber::fmt::layer()
        .with_writer(io::stderr)
        .with_target(false);
    let file_layer = write_file
        .then(|| open_log_file(paths))
        .flatten()
        .map(|file| {
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(Mutex::new(file))
                .with_target(true)
        });
    let registry = tracing_subscriber::registry().with(filter).with(stderr);
    if let Some(file_layer) = file_layer {
        let _ = registry.with(file_layer).try_init();
    } else {
        let _ = registry.try_init();
    }
}

fn open_log_file(paths: &Paths) -> Option<fs::File> {
    let dir = paths.state_dir.join("logs");
    fs::create_dir_all(&dir).ok()?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).ok()?;
    let path = dir.join("omacell.log");
    if path.exists() {
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).ok()?;
    }
    rotate_if_needed(&path).ok()?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&path)
        .ok()?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).ok()?;
    Some(file)
}

fn rotate_if_needed(path: &Path) -> io::Result<()> {
    let Ok(meta) = fs::metadata(path) else {
        return Ok(());
    };
    if meta.len() < MAX_LOG_BYTES {
        return Ok(());
    }
    let rotated = PathBuf::from(format!("{}.1", path.display()));
    let _ = fs::remove_file(&rotated);
    fs::rename(path, rotated)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::open_log_file;

    #[test]
    fn log_directory_and_existing_file_are_private() {
        let temp = tempfile::tempdir().unwrap();
        let paths = omacell_conf::Paths::from_home(temp.path());
        let dir = paths.state_dir.join("logs");
        let path = dir.join("omacell.log");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(&path, b"existing\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        drop(open_log_file(&paths).unwrap());

        assert_eq!(
            std::fs::metadata(dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
