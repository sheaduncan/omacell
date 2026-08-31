//! `omacell://<file>/card` and `omacell://<file>/<sheet>` resource URIs.

use omacell_core::error::CoreError;

use crate::error::codes;

/// Resource templates advertised by `resources/list`.
pub const RESOURCE_TEMPLATES: &[&str] = &["omacell://{file}/card", "omacell://{file}/{sheet}"];

/// Kind of an `omacell://` resource.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    /// Workbook card.
    Card {
        /// Decoded file path.
        file: String,
    },
    /// One sheet summary.
    Sheet {
        /// Decoded file path.
        file: String,
        /// Sheet name.
        sheet: String,
    },
}

/// Percent-encode a path or sheet name as a single URI segment.
#[must_use]
pub fn encode_segment(raw: &str) -> String {
    let mut out = String::new();
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte));
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn decode_segment(seg: &str) -> Result<String, CoreError> {
    let bytes = seg.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err(uri_err("truncated percent-encoding"));
            }
            let hi = from_hex(bytes[i + 1])?;
            let lo = from_hex(bytes[i + 2])?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| uri_err("resource URI is not UTF-8"))
}

fn from_hex(b: u8) -> Result<u8, CoreError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(uri_err("invalid percent-encoding")),
    }
}

fn uri_err(message: &str) -> CoreError {
    CoreError::new(codes::MCP_URI, message)
        .with_hint("use omacell://<percent-encoded-path>/card or …/<sheet>")
}

/// Card resource for an open file.
#[must_use]
pub fn card_uri(file: &str) -> String {
    format!("omacell://{}/card", encode_segment(file))
}

/// Sheet resource for an open file.
#[must_use]
pub fn sheet_uri(file: &str, sheet: &str) -> String {
    // `card` is the reserved resource name. Keep a worksheet with that exact
    // name addressable by using an equivalent, non-canonical percent encoding.
    let encoded_sheet = if sheet == "card" {
        "%63ard".to_string()
    } else {
        encode_segment(sheet)
    };
    format!("omacell://{}/{}", encode_segment(file), encoded_sheet)
}

/// Parse `omacell://<file>/card` or `omacell://<file>/<sheet>`.
pub fn parse_resource_uri(uri: &str) -> Result<ResourceKind, CoreError> {
    let rest = uri
        .strip_prefix("omacell://")
        .ok_or_else(|| uri_err("resource URI must start with omacell://"))?;
    let (file_seg, tail) = rest
        .split_once('/')
        .ok_or_else(|| uri_err("resource URI must be omacell://<file>/card or …/<sheet>"))?;
    if file_seg.is_empty() || tail.is_empty() || tail.contains('/') {
        return Err(uri_err(
            "resource URI must be omacell://<file>/card or …/<sheet>",
        ));
    }
    let file = decode_segment(file_seg)?;
    if tail == "card" {
        return Ok(ResourceKind::Card { file });
    }
    Ok(ResourceKind::Sheet {
        file,
        sheet: decode_segment(tail)?,
    })
}

/// Catalog resource templates.
#[must_use]
pub fn templates_json() -> Vec<serde_json::Value> {
    RESOURCE_TEMPLATES
        .iter()
        .map(|uri| serde_json::json!({"uriTemplate": uri}))
        .collect()
}
