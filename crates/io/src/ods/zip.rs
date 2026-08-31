//! Bounded ODS zip (same size/ratio caps as OPC).

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use omacell_core::error::CoreError;
use zip::ZipArchive;

use crate::error;
use crate::xlsx::opc::{
    MAX_ENTRY_BYTES, MAX_PACKAGE_BYTES, MAX_UNCOMPRESSED_TOTAL, MAX_ZIP_ENTRIES, sanitize_path,
};

/// Named zip parts.
pub(super) fn read_parts(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, CoreError> {
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(error::xlsx_limit(format!(
            "compressed package is {} bytes; maximum is {MAX_PACKAGE_BYTES}",
            bytes.len()
        )));
    }
    let mut zip =
        ZipArchive::new(Cursor::new(bytes)).map_err(|e| error::ods_format(e.to_string()))?;
    if zip.len() > MAX_ZIP_ENTRIES {
        return Err(error::xlsx_limit(format!(
            "zip has {} entries; maximum is {MAX_ZIP_ENTRIES}",
            zip.len()
        )));
    }
    let mut parts = BTreeMap::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|e| error::ods_format(e.to_string()))?;
        if file.is_dir() {
            continue;
        }
        let name = sanitize_path(file.name())?;
        let uncompressed = file.size();
        if uncompressed > MAX_ENTRY_BYTES {
            return Err(error::xlsx_limit(format!(
                "entry {name} is {uncompressed} bytes uncompressed"
            )));
        }
        total = total.saturating_add(uncompressed);
        if total > MAX_UNCOMPRESSED_TOTAL {
            return Err(error::xlsx_limit(
                "uncompressed ODS exceeds the package cap",
            ));
        }
        let mut buf = Vec::new();
        file.by_ref()
            .take(MAX_ENTRY_BYTES.saturating_add(1))
            .read_to_end(&mut buf)
            .map_err(|e| error::ods_format(e.to_string()))?;
        if buf.len() as u64 > MAX_ENTRY_BYTES {
            return Err(error::xlsx_limit(format!(
                "entry {name} exceeded the read cap"
            )));
        }
        parts.insert(name, buf);
    }
    Ok(parts)
}
