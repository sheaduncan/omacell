//! Import preview: raw vs converted, with a `changed` flag.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use omacell_core::error::CoreError;
use serde::{Deserialize, Serialize};

use super::encode::{DecodingReader, bom_len};
use super::infer::{ConvertedKind, convert_cell};
use super::plan::{DEFAULT_PREVIEW_ROWS, ImportPlan};
use super::records::{FieldLimitReader, reader_builder, record_to_row};
use crate::error;

/// One preview cell.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewCell {
    /// Original field text.
    pub raw: String,
    /// Display of the value that would be stored.
    pub would_become: String,
    /// Converted kind (`text`, `number`, `bool`, `date`, `empty`).
    pub kind: String,
    /// True when the stored type is number, bool, or date.
    pub changed: bool,
}

/// Header plus `n` data rows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewRows {
    /// Header cells when `plan.has_header`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<Vec<String>>,
    /// Converted data rows.
    pub rows: Vec<Vec<PreviewCell>>,
}

impl PreviewCell {
    fn from_raw(raw: &str, col: usize, plan: &ImportPlan) -> Self {
        let converted = convert_cell(raw, plan.column_type(col), plan);
        Self {
            raw: raw.to_string(),
            would_become: converted.preview_text(plan),
            kind: match converted.kind() {
                ConvertedKind::Empty => "empty",
                ConvertedKind::Number => "number",
                ConvertedKind::Bool => "bool",
                ConvertedKind::Date => "date",
                ConvertedKind::Text => "text",
            }
            .to_string(),
            changed: converted.changed(),
        }
    }
}

/// Preview the first `n` data rows of `bytes` (0 means [`DEFAULT_PREVIEW_ROWS`]).
pub fn preview(bytes: &[u8], plan: &ImportPlan, n: usize) -> Result<PreviewRows, CoreError> {
    plan.validate()?;
    let skip = bom_len(plan.encoding, bytes);
    let decoded = DecodingReader::new(bytes, plan.encoding, skip);
    preview_reader(decoded, plan, n)
}

/// Preview a path.
pub fn preview_path(path: &Path, plan: &ImportPlan, n: usize) -> Result<PreviewRows, CoreError> {
    plan.validate()?;
    let file = std::fs::File::open(path).map_err(|e| error::parse(e.to_string()))?;
    let mut buffered = BufReader::new(file);
    let skip = bom_len(
        plan.encoding,
        buffered
            .fill_buf()
            .map_err(|e| error::parse(e.to_string()))?,
    );
    let decoded = DecodingReader::new(buffered, plan.encoding, skip);
    preview_reader(decoded, plan, n)
}

fn preview_reader<R: Read>(
    reader: R,
    plan: &ImportPlan,
    n: usize,
) -> Result<PreviewRows, CoreError> {
    let limited = FieldLimitReader::new(reader, plan)?;
    let mut rdr = reader_builder(plan)?.from_reader(limited);
    let mut records = rdr.records();
    for _ in 0..plan.skip_rows {
        let Some(rec) = records.next() else {
            break;
        };
        record_to_row(&rec.map_err(super::records::map_csv)?)?;
    }
    let header = if plan.has_header {
        match records.next() {
            Some(rec) => Some(record_to_row(&rec.map_err(super::records::map_csv)?)?),
            None => Some(Vec::new()),
        }
    } else {
        None
    };
    let take = if n == 0 { DEFAULT_PREVIEW_ROWS } else { n };
    let mut rows = Vec::with_capacity(take);
    for rec in records.take(take) {
        let rec = record_to_row(&rec.map_err(super::records::map_csv)?)?;
        let mut row = Vec::with_capacity(rec.len());
        for (i, raw) in rec.iter().enumerate() {
            row.push(PreviewCell::from_raw(raw, i, plan));
        }
        rows.push(row);
    }
    Ok(PreviewRows { header, rows })
}

/// Convert a single already-split row (clipboard / tests).
#[must_use]
pub fn preview_row(fields: &[String], plan: &ImportPlan) -> Vec<PreviewCell> {
    fields
        .iter()
        .enumerate()
        .map(|(i, raw)| PreviewCell::from_raw(raw, i, plan))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::{self, Read};

    use super::*;

    struct FailAfter {
        data: Vec<u8>,
        pos: usize,
        fail_at: usize,
    }

    impl Read for FailAfter {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.pos >= self.fail_at {
                return Err(io::Error::other("preview read past requested rows"));
            }
            let end = self.data.len().min(self.fail_at).min(self.pos + buf.len());
            let len = end - self.pos;
            buf[..len].copy_from_slice(&self.data[self.pos..end]);
            self.pos = end;
            Ok(len)
        }
    }

    #[test]
    fn preview_stops_after_requested_rows() {
        let data = "1,2\n".repeat(250_000).into_bytes();
        let reader = FailAfter {
            data,
            pos: 0,
            fail_at: 128 * 1024,
        };
        let preview = preview_reader(reader, &ImportPlan::default(), 1).unwrap();
        assert_eq!(preview.rows.len(), 1);
    }
}
