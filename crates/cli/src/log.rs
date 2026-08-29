//! Tracing to stderr and `~/.local/state/omacell/logs/`.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

use omacell_conf::Paths;

const MAX_LOG_BYTES: u64 = 1024 * 1024;

/// Install stderr + rotating file logging.
pub fn init(paths: &Paths, verbose: u8, quiet: bool) {
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
    let file_layer = open_log_file(paths).map(|file| {
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
    let path = dir.join("omacell.log");
    rotate_if_needed(&path).ok()?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()
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
