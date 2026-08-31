//! OpenDocument Spreadsheet (`.ods`) read and basic write (F-9.5).

mod read;
mod write;
mod zip;

use std::path::Path;

use omacell_core::error::CoreError;
use omacell_core::workbook::Workbook;

use crate::error;
use crate::xlsx::{acquire_lock, peer_lock_blocks, release_lock};

/// Open an `.ods` path.
pub fn open(path: &Path) -> Result<Workbook, CoreError> {
    peer_lock_blocks(path)?;
    let bytes = std::fs::read(path).map_err(|e| error::ods_format(e.to_string()))?;
    open_bytes(&bytes)
}

/// Open ODS bytes.
pub fn open_bytes(bytes: &[u8]) -> Result<Workbook, CoreError> {
    read::open_bytes(bytes)
}

/// Save a workbook as ODS.
pub fn save(wb: &Workbook, path: &Path) -> Result<(), CoreError> {
    peer_lock_blocks(path)?;
    let bytes = save_bytes(wb)?;
    let _ = acquire_lock(path);
    let result = std::fs::write(path, bytes).map_err(|e| error::ods_format(e.to_string()));
    let _ = release_lock(path);
    result
}

/// Encode ODS bytes (no lock).
pub fn save_bytes(wb: &Workbook) -> Result<Vec<u8>, CoreError> {
    write::save_bytes(wb)
}
