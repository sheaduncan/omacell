//! `.omc` text workbook and changeset records (spec F-9.3, Appendix E).
//!
//! ```
//! use omacell_core::workbook::Workbook;
//! use omacell_io::omc::{OmcDocument, to_string, open_str};
//! let text = to_string(&OmcDocument::from_workbook(Workbook::new())).unwrap();
//! let again = open_str(&text).unwrap();
//! assert_eq!(again.workbook.sheets().count(), 1);
//! ```

mod read;
mod write;

use std::collections::HashMap;
use std::path::Path;

use omacell_core::changeset::Changeset;
use omacell_core::error::CoreError;
use omacell_core::workbook::Workbook;

use crate::error;
use crate::xlsx::{OpcPackage, WorksheetExtras, XlsxDocument};

/// Maximum `.omc` document size.
pub const MAX_OMC_BYTES: usize = 32 * 1024 * 1024;
/// Maximum length of one record line.
pub const MAX_OMC_LINE: usize = 1024 * 1024;
/// Maximum number of records (excluding comments).
pub const MAX_OMC_RECORDS: usize = 1_000_000;

/// Opened `.omc` workbook (and optional changeset).
#[derive(Clone, Debug)]
pub struct OmcDocument {
    /// Engine workbook (L1/L2 modeled fields).
    pub workbook: Workbook,
    /// Unmodeled worksheet fragments (CF/DV/print/sparklines/autofilter).
    pub extras: HashMap<String, WorksheetExtras>,
    /// Changeset body when the file is (or includes) change records.
    pub changeset: Option<Changeset>,
}

impl OmcDocument {
    /// Wrap a workbook with empty extras.
    #[must_use]
    pub fn from_workbook(workbook: Workbook) -> Self {
        Self {
            workbook,
            extras: HashMap::new(),
            changeset: None,
        }
    }
}

/// Parts that could not be represented in `.omc`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConversionReport {
    /// Human-readable dropped items (L3 binaries, non-UTF-8 custom parts, `aicache`).
    pub dropped: Vec<String>,
}

/// Open a path.
pub fn open(path: &Path) -> Result<OmcDocument, CoreError> {
    let bytes = std::fs::read(path).map_err(|e| error::omc_parse(e.to_string()))?;
    open_bytes(&bytes)
}

/// Open UTF-8 bytes.
pub fn open_bytes(bytes: &[u8]) -> Result<OmcDocument, CoreError> {
    if bytes.len() > MAX_OMC_BYTES {
        return Err(error::omc_limit(format!(
            "omc document is {} bytes; maximum is {MAX_OMC_BYTES}",
            bytes.len()
        )));
    }
    if bytes.contains(&0) {
        return Err(error::omc_parse("NUL byte in omc document"));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| error::omc_parse("omc document is not valid UTF-8"))?;
    open_str(text)
}

/// Parse a UTF-8 `.omc` string.
pub fn open_str(text: &str) -> Result<OmcDocument, CoreError> {
    validate_text(text)?;
    read::parse(text)
}

/// Encode `doc` as `.omc` text (LF, trailing newline).
pub fn to_string(doc: &OmcDocument) -> Result<String, CoreError> {
    let text = write::encode(doc)?;
    validate_text(&text)?;
    Ok(text)
}

/// Write `doc` to `path`.
pub fn write_to_path(doc: &OmcDocument, path: &Path) -> Result<(), CoreError> {
    let text = to_string(doc)?;
    std::fs::write(path, text).map_err(|e| error::omc_parse(e.to_string()))
}

/// Convert an opened `.xlsx` into `.omc`, listing unrepresentable parts.
#[must_use]
pub fn from_xlsx(doc: &XlsxDocument) -> (OmcDocument, ConversionReport) {
    write::from_xlsx(doc)
}

/// Encode a changeset as a standalone `.omc` document.
pub fn changeset_to_omc(cs: &Changeset) -> Result<String, CoreError> {
    let text = write::encode_changeset(cs)?;
    validate_text(&text)?;
    Ok(text)
}

/// Parse a changeset `.omc` (must contain `change` records).
pub fn changeset_from_omc(text: &str) -> Result<Changeset, CoreError> {
    let doc = open_str(text)?;
    let changeset = doc
        .changeset
        .ok_or_else(|| error::omc_format("omc document has no change records"))?;
    if changeset.forward.is_empty() && changeset.inverse.is_empty() {
        return Err(error::omc_format("omc document has no change records"));
    }
    Ok(changeset)
}

/// Empty OPC package for reconstructing an `.xlsx` from `.omc` (no L3 parts).
#[must_use]
pub fn empty_package() -> OpcPackage {
    OpcPackage {
        parts: indexmap::IndexMap::new(),
        package_rels: Vec::new(),
    }
}

fn validate_text(text: &str) -> Result<(), CoreError> {
    if text.len() > MAX_OMC_BYTES {
        return Err(error::omc_limit(format!(
            "omc document is {} bytes; maximum is {MAX_OMC_BYTES}",
            text.len()
        )));
    }
    if text.as_bytes().contains(&0) {
        return Err(error::omc_parse("NUL byte in omc document"));
    }
    let mut records = 0usize;
    for (index, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.len() > MAX_OMC_LINE {
            return Err(error::omc_limit(format!(
                "line {} is {} bytes; maximum is {MAX_OMC_LINE}",
                index + 1,
                line.len()
            )));
        }
        let trimmed = line.trim_start();
        if !trimmed.is_empty() && !trimmed.starts_with('#') {
            records += 1;
            if records > MAX_OMC_RECORDS {
                return Err(error::omc_limit(format!(
                    "more than {MAX_OMC_RECORDS} records"
                )));
            }
        }
    }
    Ok(())
}
