//! Import preview: raw vs converted, with a `changed` flag.

use std::path::Path;

use omacell_core::error::CoreError;
use serde::{Deserialize, Serialize};

use super::encode::{DecodingReader, decode_all};
use super::infer::{ConvertedKind, convert_cell};
use super::plan::{DEFAULT_PREVIEW_ROWS, ImportPlan};
use super::records::{collect_records, reader_builder};
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
    let text = decode_all(bytes, plan.encoding)?;
    rows_from_utf8(text.as_bytes(), plan, n)
}

/// Preview a path.
pub fn preview_path(path: &Path, plan: &ImportPlan, n: usize) -> Result<PreviewRows, CoreError> {
    plan.validate()?;
    let file = std::fs::File::open(path).map_err(|e| error::parse(e.to_string()))?;
    let skip = super::encode::plan_bom_skip(plan.encoding, plan.bom);
    let decoded = DecodingReader::new(file, plan.encoding, skip);
    let mut rdr = reader_builder(plan)?.from_reader(decoded);
    let recs = collect_records(&mut rdr)?;
    build_preview(recs, plan, n)
}

fn rows_from_utf8(utf8: &[u8], plan: &ImportPlan, n: usize) -> Result<PreviewRows, CoreError> {
    let mut rdr = reader_builder(plan)?.from_reader(utf8);
    let recs = collect_records(&mut rdr)?;
    build_preview(recs, plan, n)
}

fn build_preview(
    mut recs: Vec<Vec<String>>,
    plan: &ImportPlan,
    n: usize,
) -> Result<PreviewRows, CoreError> {
    let skip = plan.skip_rows as usize;
    if skip > recs.len() {
        recs.clear();
    } else {
        recs.drain(..skip);
    }
    let header = if plan.has_header {
        if recs.is_empty() {
            Some(Vec::new())
        } else {
            Some(recs.remove(0))
        }
    } else {
        None
    };
    let take = if n == 0 { DEFAULT_PREVIEW_ROWS } else { n };
    let mut rows = Vec::new();
    for rec in recs.into_iter().take(take) {
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
