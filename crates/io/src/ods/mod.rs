//! OpenDocument Spreadsheet (`.ods`) read and basic write (F-9.5).

mod read;
mod write;
mod zip;

use std::path::Path;

use omacell_core::error::CoreError;
use omacell_core::workbook::Workbook;

use crate::error;
use crate::xlsx::{SaveOptions, atomic_write_bytes, peer_lock_blocks};

/// Open an `.ods` path.
pub fn open(path: &Path) -> Result<Workbook, CoreError> {
    peer_lock_blocks(path)?;
    let len = std::fs::metadata(path)
        .map_err(|e| error::ods_format(e.to_string()))?
        .len();
    if len > crate::xlsx::MAX_PACKAGE_BYTES {
        return Err(error::xlsx_limit(format!(
            "compressed ODS is {len} bytes; maximum is {}",
            crate::xlsx::MAX_PACKAGE_BYTES
        )));
    }
    let bytes = std::fs::read(path).map_err(|e| error::ods_format(e.to_string()))?;
    open_bytes(&bytes)
}

/// Open ODS bytes.
pub fn open_bytes(bytes: &[u8]) -> Result<Workbook, CoreError> {
    read::open_bytes(bytes)
}

/// Save a workbook as ODS.
pub fn save(wb: &Workbook, path: &Path) -> Result<(), CoreError> {
    let bytes = save_bytes(wb)?;
    atomic_write_bytes(path, &bytes, SaveOptions::default())
}

/// Encode ODS bytes (no lock).
pub fn save_bytes(wb: &Workbook) -> Result<Vec<u8>, CoreError> {
    write::save_bytes(wb)
}
