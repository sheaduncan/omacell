//! Atomic save, backups, and LibreOffice-compatible lock files (F-9.7, §12.2).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use omacell_core::error::CoreError;

use super::XlsxDocument;
use super::write;
use crate::error;

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
    let name = xlsx
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workbook.xlsx");
    xlsx.with_file_name(format!(".~lock.{name}#"))
}

/// Create a lock file. Fails if another live owner holds it.
pub fn acquire_lock(xlsx: &Path) -> Result<PathBuf, CoreError> {
    let path = lock_path(xlsx);
    if path.exists() {
        let text = fs::read_to_string(&path).unwrap_or_default();
        if let Some(pid) = parse_lock_pid(&text)
            && pid != std::process::id()
            && process_alive(pid)
        {
            return Err(error::xlsx_lock(format!(
                "{} is locked by pid {pid}",
                path.display()
            )));
        }
        let _ = fs::remove_file(&path);
    }
    let body = format!(
        "Omacell,{},{},{},{}\n",
        whoami_user(),
        whoami_host(),
        unix_now(),
        std::process::id()
    );
    fs::write(&path, body).map_err(|e| error::xlsx_write(e.to_string()))?;
    Ok(path)
}

/// Remove a lock we own (or a stale lock).
pub fn release_lock(xlsx: &Path) -> Result<(), CoreError> {
    let path = lock_path(xlsx);
    if path.exists() {
        fs::remove_file(&path).map_err(|e| error::xlsx_write(e.to_string()))?;
    }
    Ok(())
}

/// Save `doc` to `path` (temp + fsync + rename).
pub fn save(doc: &XlsxDocument, path: &Path, opts: SaveOptions) -> Result<(), CoreError> {
    let bytes = write::save_bytes(doc)?;
    atomic_write(path, &bytes, opts.keep_backups, opts.lock, false)
}

/// Save a raw workbook (no preserved L3 package).
pub fn save_workbook(
    wb: &omacell_core::workbook::Workbook,
    path: &Path,
    opts: SaveOptions,
) -> Result<(), CoreError> {
    let bytes = write::save_workbook_bytes(wb)?;
    atomic_write(path, &bytes, opts.keep_backups, opts.lock, false)
}

/// Test helper: write a temp file then abort before rename.
#[cfg(test)]
pub fn save_fail_before_rename(path: &Path, bytes: &[u8]) -> Result<(), CoreError> {
    atomic_write(path, bytes, 0, false, true)
}

pub(crate) fn atomic_write(
    path: &Path,
    bytes: &[u8],
    keep_backups: u32,
    lock: bool,
    fail_before_rename: bool,
) -> Result<(), CoreError> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if lock {
        acquire_lock(path)?;
    }
    let result = (|| {
        if keep_backups > 0 && path.exists() {
            rotate_backups(path, keep_backups)?;
        }
        let tmp = dir.join(format!(
            ".{}.{}.tmp",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("xlsx"),
            std::process::id()
        ));
        {
            let mut f = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp)
                .map_err(|e| error::xlsx_write(e.to_string()))?;
            f.write_all(bytes)
                .map_err(|e| error::xlsx_write(e.to_string()))?;
            f.sync_all().map_err(|e| error::xlsx_write(e.to_string()))?;
        }
        if fail_before_rename {
            let _ = fs::remove_file(&tmp);
            return Err(error::xlsx_write("injected failure before rename"));
        }
        fs::rename(&tmp, path).map_err(|e| error::xlsx_write(e.to_string()))?;
        if let Ok(dirf) = File::open(dir) {
            let _ = dirf.sync_all();
        }
        Ok(())
    })();
    if lock {
        let _ = release_lock(path);
    }
    result
}

fn rotate_backups(path: &Path, keep: u32) -> Result<(), CoreError> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("book.xlsx");
    let dir = path.parent().unwrap_or(Path::new("."));
    if keep == 0 {
        return Ok(());
    }
    let oldest = dir.join(format!("{name}.bak.{keep}"));
    let _ = fs::remove_file(oldest);
    for i in (1..keep).rev() {
        let from = dir.join(format!("{name}.bak.{i}"));
        let to = dir.join(format!("{name}.bak.{}", i + 1));
        if from.exists() {
            let _ = fs::rename(from, to);
        }
    }
    fs::copy(path, dir.join(format!("{name}.bak.1")))
        .map_err(|e| error::xlsx_write(e.to_string()))?;
    Ok(())
}

fn parse_lock_pid(text: &str) -> Option<u32> {
    text.split(',').nth(4)?.trim().parse().ok()
}

fn process_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
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
        fs::write(&lock, "Omacell,other,host,1,1\n").unwrap();
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
        fs::write(&lock, "Omacell,other,host,1,4294967294\n").unwrap();
        acquire_lock(&path).unwrap();
        release_lock(&path).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn backups_rotate_numbered_copies() {
        let dir = std::env::temp_dir().join(format!("omacell-xlsx-bak-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("book.xlsx");
        fs::write(&path, b"V1").unwrap();
        atomic_write(&path, b"V2", 2, false, false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"V2");
        assert_eq!(fs::read(dir.join("book.xlsx.bak.1")).unwrap(), b"V1");
        atomic_write(&path, b"V3", 2, false, false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"V3");
        assert_eq!(fs::read(dir.join("book.xlsx.bak.1")).unwrap(), b"V2");
        assert_eq!(fs::read(dir.join("book.xlsx.bak.2")).unwrap(), b"V1");
        let _ = fs::remove_dir_all(&dir);
    }
}
