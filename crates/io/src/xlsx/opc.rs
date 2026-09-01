//! OPC zip package with size, ratio, and path limits (spec F-9.6, §12.3).

use std::collections::HashSet;
use std::io::{Cursor, Read};

use indexmap::IndexMap;
use omacell_core::error::CoreError;
use zip::ZipArchive;

use super::xml::{XmlEvent, XmlReader, attr};
use crate::error;

/// Maximum number of zip entries.
pub const MAX_ZIP_ENTRIES: usize = 16_384;
/// Maximum uncompressed size of one entry.
pub const MAX_ENTRY_BYTES: u64 = 256 * 1024 * 1024;
/// Maximum sum of uncompressed sizes.
pub const MAX_UNCOMPRESSED_TOTAL: u64 = 512 * 1024 * 1024;
/// Maximum compressed package size accepted in memory.
pub const MAX_PACKAGE_BYTES: u64 = 512 * 1024 * 1024;
/// `uncompressed / compressed` ceiling for entries with compressed size ≥ 64 bytes.
pub const MAX_COMPRESSION_RATIO: u64 = 100;
/// Tiny compressed entries are exempt from the ratio check (headers, etc.).
pub const MIN_RATIO_COMPRESSED: u64 = 64;

/// One zip part, bytes intact for WP-10 L3 re-emit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreservedPart {
    /// Zip path (`xl/workbook.xml`).
    pub name: String,
    /// Content type when known.
    pub content_type: Option<String>,
    /// Raw bytes.
    pub bytes: Vec<u8>,
}

/// One relationship.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relationship {
    /// `Id` (`rId1`).
    pub id: String,
    /// Relationship type URI.
    pub rel_type: String,
    /// Target path (package-relative after resolution).
    pub target: String,
    /// External target if `TargetMode="External"`.
    pub external: bool,
}

/// Opened OPC package.
#[derive(Clone, Debug)]
pub struct OpcPackage {
    /// Parts in zip order, keyed by normalized name.
    pub parts: IndexMap<String, PreservedPart>,
    /// Package-level relationships (`_rels/.rels`).
    pub package_rels: Vec<Relationship>,
}

impl OpcPackage {
    /// Borrow a part (case-insensitive key).
    #[must_use]
    pub fn part(&self, name: &str) -> Option<&PreservedPart> {
        let key = normalize_lookup(name);
        self.parts
            .iter()
            .find(|(k, _)| normalize_lookup(k) == key)
            .map(|(_, p)| p)
    }

    /// Relationships for a part (`xl/_rels/workbook.xml.rels`).
    pub fn rels_for(&self, part_name: &str) -> Result<Vec<Relationship>, CoreError> {
        let rels_name = rels_path(part_name);
        match self.part(&rels_name) {
            Some(p) => parse_rels(&p.bytes, &part_dir(part_name)),
            None => Ok(Vec::new()),
        }
    }

    /// Office document workbook part.
    pub fn workbook_part(&self) -> Result<&PreservedPart, CoreError> {
        let office =
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
        for rel in &self.package_rels {
            if rel.rel_type == office {
                return self.part(&rel.target).ok_or_else(|| {
                    error::xlsx_format(format!("missing workbook part {}", rel.target))
                });
            }
        }
        Err(error::xlsx_format(
            "package has no officeDocument relationship",
        ))
    }
}

/// Open bytes as an OPC zip.
pub fn open_package(bytes: &[u8]) -> Result<OpcPackage, CoreError> {
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(error::xlsx_limit(format!(
            "compressed package is {} bytes; maximum is {MAX_PACKAGE_BYTES}",
            bytes.len()
        )));
    }
    let cursor = Cursor::new(bytes);
    let mut zip = ZipArchive::new(cursor).map_err(|e| error::xlsx_zip(e.to_string()))?;
    if zip.len() > MAX_ZIP_ENTRIES {
        return Err(error::xlsx_limit(format!(
            "zip has {} entries; maximum is {MAX_ZIP_ENTRIES}",
            zip.len()
        )));
    }
    let mut parts = IndexMap::new();
    let mut part_names = HashSet::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|e| error::xlsx_zip(e.to_string()))?;
        let raw_name = file.name().to_string();
        let name = sanitize_path(&raw_name)?;
        if file.is_dir() {
            continue;
        }
        let normalized_name = normalize_lookup(&name);
        if !part_names.insert(normalized_name) {
            return Err(error::xlsx_format(format!(
                "duplicate OPC part name {name:?}"
            )));
        }
        let uncompressed = file.size();
        let compressed = file.compressed_size();
        if uncompressed > MAX_ENTRY_BYTES {
            return Err(error::xlsx_limit(format!(
                "entry {name} is {uncompressed} bytes uncompressed; maximum is {MAX_ENTRY_BYTES}"
            )));
        }
        if total.saturating_add(uncompressed) > MAX_UNCOMPRESSED_TOTAL {
            return Err(error::xlsx_limit(format!(
                "uncompressed package exceeds {MAX_UNCOMPRESSED_TOTAL} bytes"
            )));
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
        let initial_capacity = uncompressed.min(read_cap).min(1024 * 1024) as usize;
        let mut buf = Vec::with_capacity(initial_capacity);
        file.by_ref()
            .take(read_cap.saturating_add(1))
            .read_to_end(&mut buf)
            .map_err(|e| error::xlsx_zip(e.to_string()))?;
        let actual = buf.len() as u64;
        if actual > MAX_ENTRY_BYTES {
            return Err(error::xlsx_limit(format!(
                "entry {name} exceeds {MAX_ENTRY_BYTES} bytes while decompressing"
            )));
        }
        if ratio_exceeded(actual, compressed) {
            return Err(error::xlsx_limit(format!(
                "entry {name} compression ratio exceeds {MAX_COMPRESSION_RATIO}:1 while decompressing"
            )));
        }
        if actual != uncompressed {
            return Err(error::xlsx_zip(format!(
                "entry {name} declared {uncompressed} uncompressed bytes but produced {actual}"
            )));
        }
        total = total.saturating_add(actual);
        if total > MAX_UNCOMPRESSED_TOTAL {
            return Err(error::xlsx_limit(format!(
                "uncompressed package exceeds {MAX_UNCOMPRESSED_TOTAL} bytes"
            )));
        }
        parts.insert(
            name.clone(),
            PreservedPart {
                name,
                content_type: None,
                bytes: buf,
            },
        );
    }
    let package_rels = match parts.get("_rels/.rels") {
        Some(p) => parse_rels(&p.bytes, "")?,
        None => {
            return Err(error::xlsx_format("missing /_rels/.rels"));
        }
    };
    let mut pkg = OpcPackage {
        parts,
        package_rels,
    };
    apply_content_types(&mut pkg)?;
    Ok(pkg)
}

/// Reject path traversal and absolute zip names.
pub fn sanitize_path(name: &str) -> Result<String, CoreError> {
    let n = name.replace('\\', "/");
    if n.is_empty() {
        return Err(error::xlsx_path("empty zip entry name"));
    }
    if n.starts_with('/') {
        return Err(error::xlsx_path(format!("zip entry {name:?} is absolute")));
    }
    if n.split('/').any(|seg| seg == ".." || seg == ".") {
        return Err(error::xlsx_path(format!(
            "zip entry {name:?} contains a '.' or '..' segment"
        )));
    }
    if n.as_bytes().get(1) == Some(&b':') {
        return Err(error::xlsx_path(format!(
            "zip entry {name:?} looks absolute"
        )));
    }
    Ok(n.to_string())
}

fn ratio_exceeded(uncompressed: u64, compressed: u64) -> bool {
    compressed >= MIN_RATIO_COMPRESSED
        && uncompressed > compressed.saturating_mul(MAX_COMPRESSION_RATIO)
}

fn normalize_lookup(name: &str) -> String {
    name.replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase()
}

fn part_dir(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, _)) => dir.to_string(),
        None => String::new(),
    }
}

pub(crate) fn rels_path(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

/// Resolve `Target` against the source part directory.
#[must_use]
pub fn resolve_target(base_dir: &str, target: &str) -> String {
    let t = target.replace('\\', "/");
    if t.starts_with('/') {
        return t.trim_start_matches('/').to_string();
    }
    let mut stack: Vec<&str> = if base_dir.is_empty() {
        Vec::new()
    } else {
        base_dir.split('/').filter(|s| !s.is_empty()).collect()
    };
    for seg in t.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            stack.pop();
        } else {
            stack.push(seg);
        }
    }
    stack.join("/")
}

fn parse_rels(bytes: &[u8], base_dir: &str) -> Result<Vec<Relationship>, CoreError> {
    let mut r = XmlReader::new(bytes);
    let mut out = Vec::new();
    while let Some(ev) = r.next()? {
        match ev {
            XmlEvent::Start { name, attrs } | XmlEvent::Empty { name, attrs }
                if name == "Relationship" =>
            {
                let id = attr(&attrs, "Id").unwrap_or("").to_string();
                let rel_type = attr(&attrs, "Type").unwrap_or("").to_string();
                let target = attr(&attrs, "Target").unwrap_or("").to_string();
                let external =
                    attr(&attrs, "TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("External"));
                let resolved = if external {
                    target.clone()
                } else {
                    resolve_target(base_dir, &target)
                };
                out.push(Relationship {
                    id,
                    rel_type,
                    target: resolved,
                    external,
                });
            }
            _ => {}
        }
    }
    Ok(out)
}

fn apply_content_types(pkg: &mut OpcPackage) -> Result<(), CoreError> {
    let Some(ct) = pkg.part("[Content_Types].xml") else {
        return Err(error::xlsx_format("missing [Content_Types].xml"));
    };
    let bytes = ct.bytes.clone();
    let mut r = XmlReader::new(&bytes);
    let mut defaults: Vec<(String, String)> = Vec::new();
    let mut overrides: Vec<(String, String)> = Vec::new();
    while let Some(ev) = r.next()? {
        match ev {
            XmlEvent::Start { name, attrs } | XmlEvent::Empty { name, attrs } => {
                if name == "Default"
                    && let (Some(ext), Some(cty)) =
                        (attr(&attrs, "Extension"), attr(&attrs, "ContentType"))
                {
                    defaults.push((ext.to_ascii_lowercase(), cty.to_string()));
                } else if name == "Override"
                    && let (Some(part), Some(cty)) =
                        (attr(&attrs, "PartName"), attr(&attrs, "ContentType"))
                {
                    let p = part.trim_start_matches('/').to_string();
                    overrides.push((p, cty.to_string()));
                }
            }
            _ => {}
        }
    }
    for part in pkg.parts.values_mut() {
        let key = part.name.clone();
        if let Some((_, cty)) = overrides.iter().find(|(n, _)| n.eq_ignore_ascii_case(&key)) {
            part.content_type = Some(cty.clone());
            continue;
        }
        if let Some((_, ext)) = key.rsplit_once('.')
            && let Some((_, cty)) = defaults.iter().find(|(e, _)| e.eq_ignore_ascii_case(ext))
        {
            part.content_type = Some(cty.clone());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_dotdot() {
        assert!(sanitize_path("../xl/workbook.xml").is_err());
        assert!(sanitize_path("xl/../workbook.xml").is_err());
        assert!(sanitize_path("/xl/workbook.xml").is_err());
        assert!(sanitize_path("\\xl\\workbook.xml").is_err());
        assert!(sanitize_path("C:\\xl\\workbook.xml").is_err());
    }
}
