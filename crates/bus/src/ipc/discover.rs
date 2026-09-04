//! Runtime-directory bind helpers and instance discovery.

use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use omacell_core::error::CoreError;

use super::protocol::{Discovery, MAX_FRAME_BYTES, VERSION};
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
        return validate_runtime_dir(dir);
    }
    fs::DirBuilder::new()
        .mode(DIR_MODE)
        .recursive(true)
        .create(dir)
        .map_err(|err| error::ipc_socket(format!("create {}: {err}", dir.display())))?;
    fs::set_permissions(dir, fs::Permissions::from_mode(DIR_MODE))
        .map_err(|err| error::ipc_socket(format!("chmod {}: {err}", dir.display())))?;
    validate_runtime_dir(dir)
}

fn validate_runtime_dir(dir: &Path) -> Result<(), CoreError> {
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

/// Ephemeral focus marker path `{dir}/{pid}.focus`.
pub(super) fn focus_path(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("{pid}.focus"))
}

/// Publish or clear focus for one running instance.
pub(super) fn set_instance_focused(dir: &Path, pid: u32, focused: bool) -> Result<(), CoreError> {
    let path = focus_path(dir, pid);
    let existing = match fs::symlink_metadata(&path) {
        Ok(meta) => Some(meta),
        Err(err) if err.kind() == ErrorKind::NotFound => None,
        Err(err) => {
            return Err(error::ipc_socket(format!("stat {}: {err}", path.display())));
        }
    };
    if let Some(meta) = existing {
        if meta.file_type().is_symlink() || !meta.is_file() || !owned_by_self(&meta) {
            return Err(error::ipc_socket(format!(
                "{} is not a regular file owned by this user",
                path.display()
            )));
        }
        fs::remove_file(&path)
            .map_err(|err| error::ipc_socket(format!("remove {}: {err}", path.display())))?;
    }
    if !focused {
        return Ok(());
    }
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true).mode(FILE_MODE);
    let file = opts
        .open(&path)
        .map_err(|err| error::ipc_socket(format!("write {}: {err}", path.display())))?;
    file.set_permissions(fs::Permissions::from_mode(FILE_MODE))
        .map_err(|err| error::ipc_socket(format!("chmod {}: {err}", path.display())))?;
    Ok(())
}

/// Remove a leftover socket only when we own it and nothing is listening.
///
/// Pid liveness is not sufficient: after a crash the kernel can reuse the
/// pid while the previous process's socket file remains. A connect probe
/// distinguishes a live listener from that leftover.
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
    if socket_has_listener(&path) {
        return Err(error::ipc_socket(format!(
            "{} still has a live listener; not removing",
            path.display()
        )));
    }
    fs::remove_file(&path)
        .map_err(|err| error::ipc_socket(format!("remove stale {}: {err}", path.display())))?;
    remove_owned_regular_file(&instance_path(dir, pid));
    remove_owned_regular_file(&focus_path(dir, pid));
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
    if let Ok(meta) = fs::symlink_metadata(&path) {
        if !meta.is_file() || !owned_by_self(&meta) {
            return Err(error::ipc_socket(format!(
                "{} is not a regular file owned by this user",
                path.display()
            )));
        }
        fs::remove_file(&path)
            .map_err(|err| error::ipc_socket(format!("replace {}: {err}", path.display())))?;
    }
    let json = serde_json::to_vec_pretty(&record)
        .map_err(|err| error::ipc_frame(format!("discovery json: {err}")))?;
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true).mode(FILE_MODE);
    use std::io::Write;
    let mut file = opts
        .open(&path)
        .map_err(|err| error::ipc_socket(format!("write {}: {err}", path.display())))?;
    file.set_permissions(fs::Permissions::from_mode(FILE_MODE))
        .map_err(|err| error::ipc_socket(format!("chmod {}: {err}", path.display())))?;
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
        if !meta.file_type().is_socket() || !owned_by_self(&meta) {
            continue;
        }
        if !pid_is_alive(pid) || !socket_has_listener(&path) {
            let _ = remove_stale_socket(dir, pid);
            continue;
        }
        let inst = instance_path(dir, pid);
        let expected_socket = name.to_string();
        let started_unix_ms =
            read_valid_started(&inst, pid, &expected_socket).unwrap_or_else(|| {
                meta.modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0)
            });
        found.push(Discovery {
            v: VERSION,
            pid,
            socket: expected_socket,
            started_unix_ms,
        });
    }
    found.sort_by_key(|a| std::cmp::Reverse(a.started_unix_ms));
    Ok(found)
}

/// Newest live owned instance, if any.
pub fn discover_newest(dir: &Path) -> Result<Option<Discovery>, CoreError> {
    Ok(list_live_instances(dir)?.into_iter().next())
}

/// Most recently focused live owned instance, if one has published focus.
pub fn discover_focused(dir: &Path) -> Result<Option<Discovery>, CoreError> {
    let instances = list_live_instances(dir)?;
    Ok(focused_instance(dir, &instances))
}

/// Default IPC target: focused instance, falling back to the newest live one.
pub fn discover_default(dir: &Path) -> Result<Option<Discovery>, CoreError> {
    let instances = list_live_instances(dir)?;
    Ok(focused_instance(dir, &instances).or_else(|| instances.into_iter().next()))
}

/// Absolute socket path for a discovery record.
#[must_use]
pub fn discovered_socket(dir: &Path, record: &Discovery) -> PathBuf {
    socket_path(dir, record.pid)
}

fn read_valid_started(path: &Path, pid: u32, socket: &str) -> Option<u64> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || !owned_by_self(&meta) || meta.len() > MAX_FRAME_BYTES as u64 {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    let record: Discovery = serde_json::from_str(&text).ok()?;
    (record.v == VERSION && record.pid == pid && record.socket == socket)
        .then_some(record.started_unix_ms)
}

fn focused_instance(dir: &Path, instances: &[Discovery]) -> Option<Discovery> {
    instances
        .iter()
        .filter_map(|instance| {
            let meta = fs::symlink_metadata(focus_path(dir, instance.pid)).ok()?;
            let mode = meta.permissions().mode() & 0o777;
            if !meta.is_file()
                || meta.file_type().is_symlink()
                || !owned_by_self(&meta)
                || mode != FILE_MODE
                || meta.len() != 0
            {
                return None;
            }
            let modified = meta.modified().ok()?;
            Some((modified, instance.started_unix_ms, instance.pid, instance))
        })
        .max_by_key(|(modified, started, pid, _)| (*modified, *started, *pid))
        .map(|(_, _, _, instance)| instance.clone())
}

#[must_use]
pub fn pid_is_alive(pid: u32) -> bool {
    match fs::symlink_metadata(format!("/proc/{pid}")) {
        Ok(meta) => meta.is_dir(),
        Err(_) => false,
    }
}

fn socket_has_listener(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.file_type().is_socket() {
        return false;
    }
    match UnixStream::connect(path) {
        Ok(_) => true,
        Err(err) => !matches!(
            err.kind(),
            ErrorKind::ConnectionRefused | ErrorKind::NotFound
        ),
    }
}

fn remove_owned_regular_file(path: &Path) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    if meta.file_type().is_symlink() || !meta.is_file() || !owned_by_self(&meta) {
        return;
    }
    let _ = fs::remove_file(path);
}

fn owned_by_self(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    match uid() {
        Some(uid) => meta.uid() == uid,
        None => false,
    }
}
