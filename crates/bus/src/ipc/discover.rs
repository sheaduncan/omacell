//! Runtime-directory bind helpers and instance discovery.

use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use omacell_core::error::CoreError;

use super::protocol::{Discovery, VERSION};
use crate::error;

const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;

/// `$XDG_RUNTIME_DIR/omacell`, or `/tmp/omacell-<uid>` when unset.
#[must_use]
pub fn default_runtime_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR")
        && !xdg.is_empty()
    {
        return PathBuf::from(xdg).join("omacell");
    }
    PathBuf::from(format!("/tmp/omacell-{}", uid().unwrap_or(0)))
}

fn uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    // Follow `/proc/self` so we get the process uid, not the procfs symlink.
    fs::metadata("/proc/self").ok().map(|meta| meta.uid())
}

/// Prepare `dir` as a 0700, non-symlink directory owned by this user.
pub fn prepare_runtime_dir(dir: &Path) -> Result<(), CoreError> {
    if dir.exists() {
        let meta = fs::symlink_metadata(dir)
            .map_err(|err| error::ipc_socket(format!("stat {}: {err}", dir.display())))?;
        if meta.file_type().is_symlink() {
            return Err(error::ipc_socket(format!("{} is a symlink", dir.display())));
        }
        if !meta.is_dir() {
            return Err(error::ipc_socket(format!(
                "{} is not a directory",
                dir.display()
            )));
        }
        let mode = meta.permissions().mode() & 0o777;
        if mode != DIR_MODE {
            return Err(error::ipc_socket(format!(
                "{} mode is {mode:o}, expected {DIR_MODE:o}",
                dir.display()
            )));
        }
        if !owned_by_self(&meta) {
            return Err(error::ipc_socket(format!(
                "{} is not owned by this user",
                dir.display()
            )));
        }
        return Ok(());
    }
    fs::DirBuilder::new()
        .mode(DIR_MODE)
        .recursive(true)
        .create(dir)
        .map_err(|err| error::ipc_socket(format!("create {}: {err}", dir.display())))?;
    // Refuse if a symlink appeared between check and create.
    let meta = fs::symlink_metadata(dir)
        .map_err(|err| error::ipc_socket(format!("stat {}: {err}", dir.display())))?;
    if meta.file_type().is_symlink() {
        return Err(error::ipc_socket(format!("{} is a symlink", dir.display())));
    }
    Ok(())
}

/// Socket path `{dir}/{pid}.sock`.
#[must_use]
pub fn socket_path(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("{pid}.sock"))
}

/// Discovery path `{dir}/{pid}.instance`.
#[must_use]
pub fn instance_path(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("{pid}.instance"))
}

/// Remove a leftover socket only when we own it and the pid is dead.
pub fn remove_stale_socket(dir: &Path, pid: u32) -> Result<(), CoreError> {
    let path = socket_path(dir, pid);
    if !path.exists() && fs::symlink_metadata(&path).is_err() {
        return Ok(());
    }
    let meta = fs::symlink_metadata(&path)
        .map_err(|err| error::ipc_socket(format!("stat {}: {err}", path.display())))?;
    if meta.file_type().is_symlink() {
        return Err(error::ipc_socket(format!(
            "{} is a symlink",
            path.display()
        )));
    }
    if !owned_by_self(&meta) {
        return Err(error::ipc_socket(format!(
            "{} is not owned by this user",
            path.display()
        )));
    }
    if pid_is_alive(pid) {
        return Err(error::ipc_socket(format!(
            "pid {pid} is still alive; not removing {}",
            path.display()
        )));
    }
    fs::remove_file(&path)
        .map_err(|err| error::ipc_socket(format!("remove stale {}: {err}", path.display())))?;
    let inst = instance_path(dir, pid);
    if inst.exists() {
        let _ = fs::remove_file(inst);
    }
    Ok(())
}

/// Write the discovery record next to the socket.
pub fn write_discovery(dir: &Path, pid: u32) -> Result<Discovery, CoreError> {
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let record = Discovery {
        v: VERSION,
        pid,
        socket: format!("{pid}.sock"),
        started_unix_ms: started,
    };
    let path = instance_path(dir, pid);
    if fs::symlink_metadata(&path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(error::ipc_socket(format!(
            "{} is a symlink",
            path.display()
        )));
    }
    let json = serde_json::to_vec_pretty(&record)
        .map_err(|err| error::ipc_frame(format!("discovery json: {err}")))?;
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true).mode(FILE_MODE);
    use std::io::Write;
    let mut file = opts
        .open(&path)
        .map_err(|err| error::ipc_socket(format!("write {}: {err}", path.display())))?;
    file.write_all(&json)
        .map_err(|err| error::ipc_socket(format!("write {}: {err}", path.display())))?;
    Ok(record)
}

/// Live owned instances in `dir`, newest first.
pub fn list_live_instances(dir: &Path) -> Result<Vec<Discovery>, CoreError> {
    prepare_runtime_dir(dir)?;
    let mut found = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(found),
        Err(err) => {
            return Err(error::ipc_socket(format!("read {}: {err}", dir.display())));
        }
    };
    for entry in entries {
        let entry =
            entry.map_err(|err| error::ipc_socket(format!("read {}: {err}", dir.display())))?;
        let path = entry.path();
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(pid_s) = name.strip_suffix(".sock") else {
            continue;
        };
        let Ok(pid) = pid_s.parse::<u32>() else {
            continue;
        };
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() || !owned_by_self(&meta) || !pid_is_alive(pid) {
            continue;
        }
        let inst = instance_path(dir, pid);
        let record = match fs::read_to_string(&inst) {
            Ok(text) => serde_json::from_str::<Discovery>(&text).ok(),
            Err(_) => None,
        };
        found.push(
            record.unwrap_or(Discovery {
                v: VERSION,
                pid,
                socket: name.to_string(),
                started_unix_ms: meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
            }),
        );
    }
    found.sort_by_key(|a| std::cmp::Reverse(a.started_unix_ms));
    Ok(found)
}

/// Newest live owned instance, if any.
pub fn discover_newest(dir: &Path) -> Result<Option<Discovery>, CoreError> {
    Ok(list_live_instances(dir)?.into_iter().next())
}

/// Absolute socket path for a discovery record.
#[must_use]
pub fn discovered_socket(dir: &Path, record: &Discovery) -> PathBuf {
    dir.join(&record.socket)
}

#[must_use]
pub fn pid_is_alive(pid: u32) -> bool {
    match fs::symlink_metadata(format!("/proc/{pid}")) {
        Ok(meta) => meta.is_dir(),
        Err(_) => false,
    }
}

fn owned_by_self(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    match uid() {
        Some(uid) => meta.uid() == uid,
        None => true,
    }
}
