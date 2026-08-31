//! Bounded ODS zip (same size/ratio caps as OPC).

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

use omacell_core::error::CoreError;
use zip::ZipArchive;

use crate::error;
use crate::xlsx::opc::{
    MAX_COMPRESSION_RATIO, MAX_ENTRY_BYTES, MAX_PACKAGE_BYTES, MAX_UNCOMPRESSED_TOTAL,
    MAX_ZIP_ENTRIES, MIN_RATIO_COMPRESSED, sanitize_path,
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
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|e| error::ods_format(e.to_string()))?;
        if file.is_dir() {
            continue;
        }
        let name = sanitize_path(file.name())?;
        if !names.insert(name.clone()) {
            return Err(error::ods_format(format!(
                "duplicate ODS part name {name:?}"
            )));
        }
        let uncompressed = file.size();
        let compressed = file.compressed_size();
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
        if ratio_exceeded(uncompressed, compressed) {
            return Err(error::xlsx_limit(format!(
                "entry {name} compression ratio exceeds {MAX_COMPRESSION_RATIO}:1"
            )));
        }
        let ratio_cap = if compressed >= MIN_RATIO_COMPRESSED {
            compressed.saturating_mul(MAX_COMPRESSION_RATIO)
        } else {
            MAX_ENTRY_BYTES
        };
        let read_cap = MAX_ENTRY_BYTES.min(ratio_cap);
        let mut buf = Vec::with_capacity(uncompressed.min(read_cap).min(1024 * 1024) as usize);
        file.by_ref()
            .take(read_cap.saturating_add(1))
            .read_to_end(&mut buf)
            .map_err(|e| error::ods_format(e.to_string()))?;
        if buf.len() as u64 > MAX_ENTRY_BYTES {
            return Err(error::xlsx_limit(format!(
                "entry {name} exceeded the read cap"
            )));
        }
        let actual = buf.len() as u64;
        if ratio_exceeded(actual, compressed) {
            return Err(error::xlsx_limit(format!(
                "entry {name} compression ratio exceeds {MAX_COMPRESSION_RATIO}:1 while decompressing"
            )));
        }
        if actual != uncompressed {
            return Err(error::ods_format(format!(
                "entry {name} declared {uncompressed} bytes but produced {actual}"
            )));
        }
        parts.insert(name, buf);
    }
    Ok(parts)
}

fn ratio_exceeded(uncompressed: u64, compressed: u64) -> bool {
    compressed >= MIN_RATIO_COMPRESSED
        && uncompressed > compressed.saturating_mul(MAX_COMPRESSION_RATIO)
}
