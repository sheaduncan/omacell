//! Atomic save, backups, and LibreOffice-compatible lock files (F-9.7, §12.2).

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use omacell_core::error::CoreError;

use super::XlsxDocument;
use super::write;
use crate::error;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Options for [`save`].
#[derive(Clone, Debug)]
pub struct SaveOptions {
    /// How many numbered backups of the previous file to keep (`0` = none).
    pub keep_backups: u32,
    /// Create / honour `.~lock.<name>#`.
    pub lock: bool,
}

impl Default for SaveOptions {
    fn default() -> Self {
        Self {
            keep_backups: 0,
            lock: true,
        }
    }
}

/// LibreOffice-style lock path beside `xlsx`.
#[must_use]
pub fn lock_path(xlsx: &Path) -> PathBuf {
    let mut name = OsString::from(".~lock.");
    name.push(xlsx.file_name().unwrap_or(xlsx.as_os_str()));
    name.push("#");
    xlsx.with_file_name(name)
}

/// Refuse to open or save when a live foreign / LibreOffice lock is present.
pub fn peer_lock_blocks(path: &Path) -> Result<(), CoreError> {
    let lock = lock_path(path);
    if !lock.exists() {
        return Ok(());
    }
    let text = fs::read_to_string(&lock).map_err(|e| error::xlsx_lock(e.to_string()))?;
    if let Some(owner) = LockOwner::parse(&text) {
        let current = LockOwner::current();
        if owner.host == current.host && owner.pid == current.pid {
            return Ok(());
        }
    }
    Err(error::xlsx_lock(format!(
        "{} is locked by another editor",
        path.display()
    )))
}

/// Create a lock file. Fails if another live owner holds it.
pub fn acquire_lock(xlsx: &Path) -> Result<PathBuf, CoreError> {
    let path = lock_path(xlsx);
    let owner = LockOwner::current();
    let body = owner.encode();
    loop {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(write_error) = file
                    .write_all(body.as_bytes())
                    .and_then(|()| file.sync_all())
                {
                    let _ = fs::remove_file(&path);
                    return Err(error::xlsx_write(write_error.to_string()));
                }
                return Ok(path);
            }
            Err(open_error) if open_error.kind() == io::ErrorKind::AlreadyExists => {
                if fs::symlink_metadata(&path)
                    .map(|metadata| metadata.file_type().is_symlink())
                    .unwrap_or(false)
                {
                    return Err(error::xlsx_lock(format!(
                        "refusing symbolic-link lock {}",
                        path.display()
                    )));
                }
                let mut existing_file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&path)
                    .map_err(|e| {
                        error::xlsx_lock(format!("cannot inspect {}: {e}", path.display()))
                    })?;
                rustix::fs::flock(&existing_file, rustix::fs::FlockOperation::LockExclusive)
                    .map_err(|e| {
                        error::xlsx_lock(format!("cannot guard {}: {e}", path.display()))
                    })?;
                if !path_still_names_file(&path, &existing_file)? {
                    continue;
                }
                let mut text = String::new();
                existing_file.read_to_string(&mut text).map_err(|e| {
                    error::xlsx_lock(format!("cannot inspect {}: {e}", path.display()))
                })?;
                let Some(existing) = LockOwner::parse(&text) else {
                    return Err(error::xlsx_lock(format!(
                        "{} is locked by LibreOffice or another editor",
                        path.display()
                    )));
                };
                if existing.host != owner.host || process_alive(existing.pid) {
                    return Err(error::xlsx_lock(format!(
                        "{} is locked by {}@{} (pid {})",
                        path.display(),
                        existing.user,
                        existing.host,
                        existing.pid
                    )));
                }
                match fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(error::xlsx_lock(e.to_string())),
                }
            }
            Err(e) => return Err(error::xlsx_write(e.to_string())),
        }
    }
}

/// Remove a lock owned by this Omacell process.
pub fn release_lock(xlsx: &Path) -> Result<(), CoreError> {
    let path = lock_path(xlsx);
    if fs::symlink_metadata(&path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(error::xlsx_lock(format!(
            "refusing symbolic-link lock {}",
            path.display()
        )));
    }
    let mut file = match OpenOptions::new().read(true).write(true).open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(error::xlsx_write(e.to_string())),
    };
    rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive)
        .map_err(|e| error::xlsx_lock(format!("cannot guard {}: {e}", path.display())))?;
    if !path_still_names_file(&path, &file)? {
        return Err(error::xlsx_lock(format!(
            "lock {} changed while releasing it",
            path.display()
        )));
    }
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|e| error::xlsx_write(e.to_string()))?;
    let current = LockOwner::current();
    let Some(existing) = LockOwner::parse(&text) else {
        return Err(error::xlsx_lock(format!(
            "refusing to remove foreign lock {}",
            path.display()
        )));
    };
    if existing.host != current.host || existing.pid != current.pid {
        return Err(error::xlsx_lock(format!(
            "refusing to remove lock owned by {}@{} (pid {})",
            existing.user, existing.host, existing.pid
        )));
    }
    fs::remove_file(&path).map_err(|e| error::xlsx_write(e.to_string()))?;
    Ok(())
}

/// Save `doc` to `path` (temp + fsync + rename).
pub fn save(doc: &XlsxDocument, path: &Path, opts: SaveOptions) -> Result<(), CoreError> {
    let bytes = write::save_bytes(doc)?;
    atomic_write(path, &bytes, opts.keep_backups, opts.lock, false, None)
}

/// Save `doc` atomically, aborting before destination replacement when cancelled.
pub fn save_with_cancel(
    doc: &XlsxDocument,
    path: &Path,
    opts: SaveOptions,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(), CoreError> {
    if cancel.load(Ordering::SeqCst) {
        return Err(CoreError::new("task.cancelled", "operation cancelled")
            .with_hint("the destination file was left unchanged"));
    }
    let bytes = write::save_bytes(doc)?;
    atomic_write(
        path,
        &bytes,
        opts.keep_backups,
        opts.lock,
        false,
        Some(cancel),
    )
}

/// Save a raw workbook (no preserved L3 package).
pub fn save_workbook(
    wb: &omacell_core::workbook::Workbook,
    path: &Path,
    opts: SaveOptions,
) -> Result<(), CoreError> {
    let bytes = write::save_workbook_bytes(wb)?;
    atomic_write(path, &bytes, opts.keep_backups, opts.lock, false, None)
}

/// Save a raw workbook atomically, aborting before replacement when cancelled.
pub fn save_workbook_with_cancel(
    wb: &omacell_core::workbook::Workbook,
    path: &Path,
    opts: SaveOptions,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(), CoreError> {
    if cancel.load(Ordering::SeqCst) {
        return Err(CoreError::new("task.cancelled", "operation cancelled")
            .with_hint("the destination file was left unchanged"));
    }
    let bytes = write::save_workbook_bytes(wb)?;
    atomic_write(
        path,
        &bytes,
        opts.keep_backups,
        opts.lock,
        false,
        Some(cancel),
    )
}

/// Test helper: write a temp file then abort before rename.
#[cfg(test)]
pub fn save_fail_before_rename(path: &Path, bytes: &[u8]) -> Result<(), CoreError> {
    atomic_write(path, bytes, 0, false, true, None)
}

pub(crate) fn atomic_write(
    path: &Path,
    bytes: &[u8],
    keep_backups: u32,
    lock: bool,
    fail_before_rename: bool,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<(), CoreError> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if lock {
        acquire_lock(path)?;
    }
    let result = (|| {
        let (mut temp_file, tmp) = create_unique_temp(path, dir)?;
        let mut cleanup = TempCleanup::new(tmp.clone());
        temp_file
            .write_all(bytes)
            .map_err(|e| error::xlsx_write(e.to_string()))?;
        temp_file
            .sync_all()
            .map_err(|e| error::xlsx_write(e.to_string()))?;
        drop(temp_file);
        if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            return Err(CoreError::new("task.cancelled", "operation cancelled")
                .with_hint("the destination file was left unchanged"));
        }
        if fail_before_rename {
            return Err(error::xlsx_write("injected failure before rename"));
        }
        if keep_backups > 0
            && path
                .try_exists()
                .map_err(|e| error::xlsx_write(e.to_string()))?
        {
            rotate_backups(path, keep_backups)?;
        }
        fs::rename(&tmp, path).map_err(|e| error::xlsx_write(e.to_string()))?;
        cleanup.disarm();
        File::open(dir)
            .and_then(|dirf| dirf.sync_all())
            .map_err(|e| error::xlsx_write(format!("sync destination directory: {e}")))?;
        Ok(())
    })();
    if lock
        && let Err(unlock_error) = release_lock(path)
        && result.is_ok()
    {
        return Err(unlock_error);
    }
    result
}

fn rotate_backups(path: &Path, keep: u32) -> Result<(), CoreError> {
    let name = path
        .file_name()
        .ok_or_else(|| error::xlsx_write("destination has no file name"))?;
    let dir = path.parent().unwrap_or(Path::new("."));
    if keep == 0 {
        return Ok(());
    }
    let oldest = dir.join(backup_name(name, keep));
    remove_if_exists(&oldest)?;
    for i in (1..keep).rev() {
        let from = dir.join(backup_name(name, i));
        let to = dir.join(backup_name(name, i + 1));
        if from
            .try_exists()
            .map_err(|e| error::xlsx_write(e.to_string()))?
        {
            fs::rename(from, to).map_err(|e| error::xlsx_write(e.to_string()))?;
        }
    }
    fs::copy(path, dir.join(backup_name(name, 1))).map_err(|e| error::xlsx_write(e.to_string()))?;
    Ok(())
}

fn backup_name(name: &std::ffi::OsStr, index: u32) -> OsString {
    let mut backup = name.to_os_string();
    backup.push(format!(".bak.{index}"));
    backup
}

fn remove_if_exists(path: &Path) -> Result<(), CoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(error::xlsx_write(e.to_string())),
    }
}

fn create_unique_temp(path: &Path, dir: &Path) -> Result<(File, PathBuf), CoreError> {
    loop {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut name = OsString::from(".");
        name.push(path.file_name().unwrap_or(path.as_os_str()));
        name.push(format!(".{}.{sequence}.tmp", std::process::id()));
        let temp_path = dir.join(name);
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((file, temp_path)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(error::xlsx_write(e.to_string())),
        }
    }
}

fn process_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(unix)]
fn path_still_names_file(path: &Path, file: &File) -> Result<bool, CoreError> {
    let opened = file
        .metadata()
        .map_err(|e| error::xlsx_lock(e.to_string()))?;
    match fs::metadata(path) {
        Ok(current) => Ok(opened.dev() == current.dev() && opened.ino() == current.ino()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(error::xlsx_lock(e.to_string())),
    }
}

#[cfg(not(unix))]
fn path_still_names_file(path: &Path, _file: &File) -> Result<bool, CoreError> {
    path.try_exists()
        .map_err(|e| error::xlsx_lock(e.to_string()))
}

#[derive(Debug, PartialEq, Eq)]
struct LockOwner {
    user: String,
    host: String,
    created: u64,
    pid: u32,
}

impl LockOwner {
    fn current() -> Self {
        Self {
            user: whoami_user(),
            host: whoami_host(),
            created: unix_now(),
            pid: std::process::id(),
        }
    }

    fn parse(text: &str) -> Option<Self> {
        let fields = parse_lock_fields(text)?;
        if fields[0] != "Omacell" {
            return None;
        }
        let marker = fields[4].strip_prefix("vnd.omacell.lock://")?;
        let (created, pid) = marker.split_once('/')?;
        Some(Self {
            user: fields[1].clone(),
            host: fields[2].clone(),
            created: created.parse().ok()?,
            pid: pid.parse().ok()?,
        })
    }

    fn encode(&self) -> String {
        // LibreOffice fields: office user, system user, host, edit time, user URL.
        format!(
            "Omacell,{},{},{},vnd.omacell.lock://{}/{};",
            escape_lock_field(&self.user),
            escape_lock_field(&self.host),
            self.created,
            self.created,
            self.pid
        )
    }
}

fn escape_lock_field(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '\\' | ',' | ';') {
            escaped.push('\\');
        }
        if !matches!(ch, '\n' | '\r' | '\0') {
            escaped.push(ch);
        }
    }
    escaped
}

fn parse_lock_fields(text: &str) -> Option<[String; 5]> {
    let mut fields: Vec<String> = Vec::with_capacity(5);
    let mut field = String::new();
    let mut escaped = false;
    for (offset, ch) in text.char_indices() {
        if escaped {
            if !matches!(ch, '\\' | ',' | ';') {
                return None;
            }
            field.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == ',' {
            fields.push(std::mem::take(&mut field));
        } else if ch == ';' {
            fields.push(field);
            if fields.len() != 5 || !text[offset + ch.len_utf8()..].trim().is_empty() {
                return None;
            }
            return fields.try_into().ok();
        } else {
            field.push(ch);
        }
    }
    None
}

struct TempCleanup {
    path: PathBuf,
    armed: bool,
}

impl TempCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempCleanup {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn whoami_user() -> String {
    std::env::var("USER").unwrap_or_else(|_| "omacell".into())
}

fn whoami_host() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "localhost".into())
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_path_matches_libreoffice_shape() {
        let p = Path::new("/tmp/Book.xlsx");
        assert_eq!(lock_path(p), PathBuf::from("/tmp/.~lock.Book.xlsx#"));
    }

    #[test]
    fn crash_before_rename_leaves_original() {
        let dir = std::env::temp_dir().join(format!("omacell-xlsx-crash-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("book.xlsx");
        fs::write(&path, b"ORIGINAL").unwrap();
        let err = save_fail_before_rename(&path, b"NEW-BYTES").unwrap_err();
        assert_eq!(err.code, crate::error::codes::XLSX_WRITE);
        assert_eq!(fs::read(&path).unwrap(), b"ORIGINAL");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_foreign_lock_is_refused() {
        let dir = std::env::temp_dir().join(format!("omacell-xlsx-lock-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("book.xlsx");
        fs::write(&path, b"x").unwrap();
        let lock = lock_path(&path);
        fs::write(
            &lock,
            LockOwner {
                user: "other".into(),
                host: "other-host".into(),
                created: 1,
                pid: 1,
            }
            .encode(),
        )
        .unwrap();
        let err = acquire_lock(&path).unwrap_err();
        assert_eq!(err.code, crate::error::codes::XLSX_LOCK);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_lock_is_replaced() {
        let dir = std::env::temp_dir().join(format!("omacell-xlsx-stale-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("book.xlsx");
        fs::write(&path, b"x").unwrap();
        let lock = lock_path(&path);
        fs::write(
            &lock,
            LockOwner {
                user: "other".into(),
                host: whoami_host(),
                created: 1,
                pid: 4_294_967_294,
            }
            .encode(),
        )
        .unwrap();
        acquire_lock(&path).unwrap();
        release_lock(&path).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn foreign_editor_lock_is_never_deleted() {
        let dir = std::env::temp_dir().join(format!("omacell-xlsx-lo-lock-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("book.xlsx");
        let lock = lock_path(&path);
        let libreoffice = "user,host,file:///config,file:///book.xlsx,28.08.2026 12:00;";
        fs::write(&lock, libreoffice).unwrap();
        assert_eq!(
            acquire_lock(&path).unwrap_err().code,
            crate::error::codes::XLSX_LOCK
        );
        assert_eq!(fs::read_to_string(&lock).unwrap(), libreoffice);
        assert_eq!(
            release_lock(&path).unwrap_err().code,
            crate::error::codes::XLSX_LOCK
        );
        assert_eq!(fs::read_to_string(&lock).unwrap(), libreoffice);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_process_cannot_acquire_the_same_lock_twice() {
        let dir =
            std::env::temp_dir().join(format!("omacell-xlsx-own-lock-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("book.xlsx");
        acquire_lock(&path).unwrap();
        assert_eq!(
            acquire_lock(&path).unwrap_err().code,
            crate::error::codes::XLSX_LOCK
        );
        release_lock(&path).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn lock_body_uses_libreoffice_five_field_format() {
        let owner = LockOwner {
            user: "a,b;c\\d".into(),
            host: "host".into(),
            created: 42,
            pid: 7,
        };
        let encoded = owner.encode();
        assert!(encoded.ends_with(';'));
        assert_eq!(parse_lock_fields(&encoded).unwrap().len(), 5);
        assert_eq!(LockOwner::parse(&encoded), Some(owner));
    }

    #[test]
    fn concurrent_stale_lock_recovery_has_one_winner() {
        let dir =
            std::env::temp_dir().join(format!("omacell-xlsx-lock-race-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("book.xlsx");
        fs::write(
            lock_path(&path),
            LockOwner {
                user: "other".into(),
                host: whoami_host(),
                created: 1,
                pid: 4_294_967_294,
            }
            .encode(),
        )
        .unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let path = path.clone();
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    acquire_lock(&path)
                })
            })
            .collect();
        barrier.wait();
        let winners = handles
            .into_iter()
            .map(|handle| handle.join().unwrap().is_ok())
            .filter(|won| *won)
            .count();
        assert_eq!(winners, 1);
        release_lock(&path).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn backups_rotate_numbered_copies() {
        let dir = std::env::temp_dir().join(format!("omacell-xlsx-bak-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("book.xlsx");
        fs::write(&path, b"V1").unwrap();
        atomic_write(&path, b"V2", 2, false, false, None).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"V2");
        assert_eq!(fs::read(dir.join("book.xlsx.bak.1")).unwrap(), b"V1");
        atomic_write(&path, b"V3", 2, false, false, None).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"V3");
        assert_eq!(fs::read(dir.join("book.xlsx.bak.1")).unwrap(), b"V2");
        assert_eq!(fs::read(dir.join("book.xlsx.bak.2")).unwrap(), b"V1");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_write_removes_the_unique_temp_file() {
        let dir = std::env::temp_dir().join(format!("omacell-xlsx-temp-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("book.xlsx");
        fs::write(&path, b"ORIGINAL").unwrap();
        save_fail_before_rename(&path, b"NEW").unwrap_err();
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
