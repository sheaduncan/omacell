//! Shared csv-crate reader/writer configuration.

use std::io::Read;

use omacell_core::error::CoreError;

use super::plan::{ImportPlan, MAX_FIELD_BYTES};
use crate::error;

pub(crate) fn reader_builder(plan: &ImportPlan) -> Result<::csv::ReaderBuilder, CoreError> {
    plan.validate()?;
    let mut b = ::csv::ReaderBuilder::new();
    b.delimiter(plan.delimiter_byte()?)
        .quote(plan.quote_byte()?)
        .has_headers(false)
        .flexible(true)
        .terminator(::csv::Terminator::CRLF)
        .buffer_capacity(MAX_FIELD_BYTES);
    Ok(b)
}

/// Parse every record from already-decoded UTF-8 bytes.
pub(crate) fn parse_records(utf8: &[u8], plan: &ImportPlan) -> Result<Vec<Vec<String>>, CoreError> {
    let mut rdr = reader_builder(plan)?.from_reader(utf8);
    collect_records(&mut rdr)
}

pub(crate) fn collect_records<R: Read>(
    rdr: &mut ::csv::Reader<R>,
) -> Result<Vec<Vec<String>>, CoreError> {
    let mut rows = Vec::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| error::parse(e.to_string()))?;
        if rec.len() == 1 && rec.get(0).is_some_and(|s| s.is_empty()) && rows.is_empty() {
            // keep empty records; a trailing newline after the last row is
            // an empty record we drop at the end instead.
        }
        let mut row = Vec::with_capacity(rec.len());
        for field in rec.iter() {
            if field.len() > MAX_FIELD_BYTES {
                return Err(error::limit(format!(
                    "field is {} bytes; maximum is {MAX_FIELD_BYTES}",
                    field.len()
                )));
            }
            row.push(field.to_string());
        }
        rows.push(row);
    }
    while rows.last().is_some_and(|r| r.len() == 1 && r[0].is_empty()) {
        rows.pop();
    }
    Ok(rows)
}

pub(crate) fn map_csv(err: ::csv::Error) -> CoreError {
    error::parse(err.to_string())
}
