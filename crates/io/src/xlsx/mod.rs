//! `.xlsx` / `.xlsm` reader and writer (spec F-9.1, F-9.2, F-9.6, F-9.7).
//!
//! Unknown parts stay on [`OpcPackage`] and are re-emitted byte-identical on
//! save. VBA is preserved and never executed. Modeled parts are regenerated
//! from the workbook.
//!
//! ```
//! use omacell_io::xlsx::{diff, open, save_bytes};
//! # let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus/xlsx/l1_values.xlsx"));
//! let doc = open(path).unwrap();
//! let bytes = save_bytes(&doc).unwrap();
//! let again = omacell_io::xlsx::open_bytes(&bytes).unwrap();
//! assert!(diff(&doc, &again).empty);
//! ```

mod atomic;
mod diff;
mod opc;
mod read;
mod warnings;
mod write;
mod xml;

pub use atomic::{
    SaveOptions, acquire_lock, lock_path, release_lock, save, save_with_cancel, save_workbook,
    save_workbook_with_cancel,
};
pub use diff::{DiffReport, diff};
pub use opc::{
    MAX_COMPRESSION_RATIO, MAX_ENTRY_BYTES, MAX_PACKAGE_BYTES, MAX_UNCOMPRESSED_TOTAL,
    MAX_ZIP_ENTRIES, OpcPackage, PreservedPart, Relationship, open_package, sanitize_path,
};
pub use read::WorksheetExtras;
pub use warnings::{FileWarning, FileWarnings};
pub use write::{save_bytes, save_workbook_bytes};
pub use xml::MAX_XML_DEPTH;

use std::collections::HashMap;
use std::path::Path;

use omacell_core::error::CoreError;
use omacell_core::workbook::Workbook;

use crate::error;

/// Opened workbook plus preserved package bytes and warnings.
#[derive(Clone, Debug)]
pub struct XlsxDocument {
    /// Engine workbook (L1 values/formulas/formats and modeled L2).
    pub workbook: Workbook,
    /// Recoverable issues (unparsable formulas, skipped parts, …).
    pub warnings: FileWarnings,
    /// Original OPC parts for WP-10.
    pub package: OpcPackage,
    /// Unmodeled worksheet fragments (CF, DV, print, sparklines) for WP-10.
    pub extras: HashMap<String, WorksheetExtras>,
}

/// Open a path.
pub fn open(path: &Path) -> Result<XlsxDocument, CoreError> {
    let len = std::fs::metadata(path)
        .map_err(|e| error::xlsx_zip(e.to_string()))?
        .len();
    if len > MAX_PACKAGE_BYTES {
        return Err(error::xlsx_limit(format!(
            "compressed package is {len} bytes; maximum is {MAX_PACKAGE_BYTES}"
        )));
    }
    let bytes = std::fs::read(path).map_err(|e| error::xlsx_zip(e.to_string()))?;
    open_bytes(&bytes)
}

/// Open in-memory bytes.
pub fn open_bytes(bytes: &[u8]) -> Result<XlsxDocument, CoreError> {
    read::load(bytes)
}
