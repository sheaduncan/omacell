//! Delimited export.

use std::io::{self, Write};

use omacell_core::addr::{ParsedRef, RangeRef, RefKind, SheetId, parse_a1};
use omacell_core::error::CoreError;
use omacell_core::numfmt::{FormatOptions, FormatValue, format_with};
use omacell_core::storage::UsedRange;
use omacell_core::style::NumFmtId;
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;

use super::encode::encode_all;
use super::plan::{
    ExportPlan, FormulaTextPolicy, LineEnding, MAX_BUFFERED_EXPORT_BYTES, MAX_EXPORT_RECORD_BYTES,
    MAX_FIELD_BYTES, Quoting, TextEncoding, ValueMode,
};
use crate::error;

/// Export `wb` to a bounded in-memory buffer.
///
/// Use [`export_write`] when output may exceed [`MAX_BUFFERED_EXPORT_BYTES`].
pub fn export(wb: &Workbook, plan: &ExportPlan) -> Result<Vec<u8>, CoreError> {
    let mut output = LimitedBuffer::new(MAX_BUFFERED_EXPORT_BYTES);
    match export_write(wb, plan, &mut output) {
        Err(_) if output.exceeded => Err(error::limit(format!(
            "buffered export exceeds {MAX_BUFFERED_EXPORT_BYTES} bytes; use export_write"
        ))),
        Err(err) => Err(err),
        Ok(()) => Ok(output.bytes),
    }
}

/// Export `wb` to `dest`, retaining at most one encoded record in memory.
///
/// On an error, `dest` may contain a valid prefix of the export.
pub fn export_write<W: Write>(
    wb: &Workbook,
    plan: &ExportPlan,
    mut dest: W,
) -> Result<(), CoreError> {
    plan.validate()?;
    let sheet = resolve_sheet(wb, plan.sheet.as_deref())?;
    let bounds = export_bounds(wb, sheet, plan.range.as_deref())?;
    write_bom(&mut dest, plan)?;

    match plan.encoding {
        TextEncoding::Utf8 => {
            let mut writer = writer_builder(plan)?.from_writer(&mut dest);
            if let Some((r0, c0, r1, c1)) = bounds {
                for row in r0..=r1 {
                    let record = export_record(wb, sheet, row, c0, c1, plan)?;
                    writer
                        .write_record(&record)
                        .map_err(|err| error::export(err.to_string()))?;
                }
            }
            writer
                .flush()
                .map_err(|err| error::export(err.to_string()))?;
        }
        encoding => {
            if let Some((r0, c0, r1, c1)) = bounds {
                for row in r0..=r1 {
                    let record = export_record(wb, sheet, row, c0, c1, plan)?;
                    let mut utf8 = Vec::new();
                    {
                        let mut writer = writer_builder(plan)?.from_writer(&mut utf8);
                        writer
                            .write_record(&record)
                            .map_err(|err| error::export(err.to_string()))?;
                        writer
                            .flush()
                            .map_err(|err| error::export(err.to_string()))?;
                    }
                    let text = std::str::from_utf8(&utf8)
                        .map_err(|_| error::export("internal UTF-8 export failed"))?;
                    let encoded = encode_all(text, encoding, false)?;
                    dest.write_all(&encoded)
                        .map_err(|err| error::export(err.to_string()))?;
                }
            }
        }
    }
    Ok(())
}

fn writer_builder(plan: &ExportPlan) -> Result<::csv::WriterBuilder, CoreError> {
    let mut builder = ::csv::WriterBuilder::new();
    builder
        .delimiter(plan.delimiter_byte()?)
        .quote(plan.quote_byte()?)
        .quote_style(match plan.quoting {
            Quoting::Necessary => ::csv::QuoteStyle::Necessary,
            Quoting::Always => ::csv::QuoteStyle::Always,
            Quoting::Never => ::csv::QuoteStyle::Never,
        })
        .terminator(match plan.line_ending {
            LineEnding::CrLf => ::csv::Terminator::CRLF,
            LineEnding::Lf => ::csv::Terminator::Any(b'\n'),
            LineEnding::Cr => ::csv::Terminator::Any(b'\r'),
        });
    Ok(builder)
}

fn write_bom<W: Write>(dest: &mut W, plan: &ExportPlan) -> Result<(), CoreError> {
    let bom = match plan.encoding {
        TextEncoding::Utf8 if plan.bom => &b"\xEF\xBB\xBF"[..],
        TextEncoding::Utf16Le => &b"\xFF\xFE"[..],
        TextEncoding::Utf16Be => &b"\xFE\xFF"[..],
        TextEncoding::Utf8 | TextEncoding::Latin1 => &[][..],
    };
    dest.write_all(bom)
        .map_err(|err| error::export(err.to_string()))
}

fn export_record(
    wb: &Workbook,
    sheet: SheetId,
    row: u32,
    c0: u16,
    c1: u16,
    plan: &ExportPlan,
) -> Result<Vec<String>, CoreError> {
    let mut record = Vec::with_capacity((u32::from(c1 - c0) + 1) as usize);
    let mut record_bytes = 0usize;
    for col in c0..=c1 {
        let field = cell_text(wb, sheet, row, col, plan)?;
        record_bytes = checked_record_bytes(record_bytes, field.len(), row)?;
        record.push(field);
    }
    if plan.quoting == Quoting::Never
        && let Some((column, field)) = record.iter().enumerate().find(|(_, field)| {
            field.contains(plan.delimiter)
                || field.contains(plan.quote)
                || field.contains(['\r', '\n'])
        })
    {
        return Err(error::export(format!(
            "cell at row {}, column {} requires quoting: {field:?}",
            row + 1,
            u32::from(c0) + column as u32 + 1
        )));
    }
    Ok(record)
}

fn resolve_sheet(wb: &Workbook, name: Option<&str>) -> Result<SheetId, CoreError> {
    match name {
        None => Ok(wb.active_sheet()),
        Some(n) => wb.resolve_sheet_name(n),
    }
}

fn export_bounds(
    wb: &Workbook,
    sheet: SheetId,
    range: Option<&str>,
) -> Result<Option<(u32, u16, u32, u16)>, CoreError> {
    if let Some(text) = range {
        return Ok(Some(bounds_from_a1(text)?));
    }
    match wb.used_range(sheet)? {
        None => Ok(None),
        Some(UsedRange {
            min_row,
            min_col,
            max_row,
            max_col,
        }) => Ok(Some((min_row, min_col, max_row, max_col))),
    }
}

fn bounds_from_a1(text: &str) -> Result<(u32, u16, u32, u16), CoreError> {
    let parsed: ParsedRef = parse_a1(text)?;
    match parsed.kind {
        RefKind::Cell(c) => Ok((c.row, c.col, c.row, c.col)),
        RefKind::Range(r) => Ok(range_bounds(r)),
    }
}

fn range_bounds(r: RangeRef) -> (u32, u16, u32, u16) {
    let r0 = r.start.row.min(r.end.row);
    let r1 = r.start.row.max(r.end.row);
    let c0 = r.start.col.min(r.end.col);
    let c1 = r.start.col.max(r.end.col);
    (r0, c0, r1, c1)
}

fn cell_text(
    wb: &Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
    plan: &ExportPlan,
) -> Result<String, CoreError> {
    let Some(slot) = wb.get(sheet, row, col)? else {
        return Ok(String::new());
    };
    if plan.values == ValueMode::Formulas
        && let Some(fid) = slot.formula
        && let Some(src) = wb.intern().formulas.get(fid)
    {
        return Ok(src.to_string());
    }
    match slot.value {
        Value::Empty => Ok(String::new()),
        Value::Bool(true) => Ok("TRUE".into()),
        Value::Bool(false) => Ok("FALSE".into()),
        Value::Error(e) => Ok(e.as_str().to_string()),
        Value::Text(id) => formula_safe_text(
            wb.intern().strings.get(id).unwrap_or(""),
            row,
            col,
            plan.formula_text,
        ),
        Value::Array(_) => Ok(String::new()),
        Value::Number(n) => {
            let num_fmt = wb
                .intern()
                .styles
                .get(slot.style)
                .map(|s| s.num_fmt)
                .unwrap_or(NumFmtId::GENERAL);
            let code = wb
                .num_fmt_code(num_fmt)
                .map(|c| c.into_owned())
                .unwrap_or_else(|| "General".into());
            let opts = FormatOptions {
                locale: plan.locale,
                date_system: wb.settings().date_system,
                width: None,
            };
            let formatted = format_with(FormatValue::Number(n), &code, &opts).text;
            if is_formula_like(&formatted) && !is_unambiguous_numeric_field(&formatted, plan.locale)
            {
                return formula_safe_text(&formatted, row, col, plan.formula_text);
            }
            check_field_bytes(formatted.len(), row, col)?;
            Ok(formatted)
        }
    }
}

fn is_unambiguous_numeric_field(text: &str, locale: omacell_core::locale::LocaleId) -> bool {
    let separators = locale.separators();
    let mut normalized = String::with_capacity(text.len());
    for ch in text.trim().chars() {
        if ch == separators.thousands {
            continue;
        }
        if ch == separators.decimal {
            normalized.push('.');
        } else if ch.is_ascii_digit() || matches!(ch, '+' | '-' | 'e' | 'E') {
            normalized.push(ch);
        } else {
            return false;
        }
    }
    normalized
        .parse::<f64>()
        .is_ok_and(|number| number.is_finite())
}

fn formula_safe_text(
    text: &str,
    row: u32,
    col: u16,
    policy: FormulaTextPolicy,
) -> Result<String, CoreError> {
    check_field_bytes(text.len(), row, col)?;
    if !is_formula_like(text) || policy == FormulaTextPolicy::Preserve {
        return Ok(text.to_string());
    }
    match policy {
        FormulaTextPolicy::Reject => Err(error::export(format!(
            "field at row {}, column {} could execute as a spreadsheet formula",
            row + 1,
            u32::from(col) + 1
        ))
        .with_hint("set formula_text to preserve only for trusted consumers, or to escape")),
        FormulaTextPolicy::Escape => Ok(format!("'{text}")),
        FormulaTextPolicy::Preserve => Ok(text.to_string()),
    }
}

fn is_formula_like(text: &str) -> bool {
    let trimmed = text.trim_start_matches([' ', '\t', '\r', '\n']);
    trimmed
        .chars()
        .next()
        .is_some_and(|first| matches!(first, '=' | '+' | '-' | '@'))
}

fn check_field_bytes(bytes: usize, row: u32, col: u16) -> Result<(), CoreError> {
    if bytes > MAX_FIELD_BYTES {
        return Err(error::limit(format!(
            "field at row {}, column {} is {bytes} bytes; maximum is {MAX_FIELD_BYTES}",
            row + 1,
            u32::from(col) + 1
        )));
    }
    Ok(())
}

fn checked_record_bytes(current: usize, field: usize, row: u32) -> Result<usize, CoreError> {
    let bytes = current
        .checked_add(field)
        .ok_or_else(|| error::limit("export record size overflow"))?;
    if bytes > MAX_EXPORT_RECORD_BYTES {
        return Err(error::limit(format!(
            "export row {} contains more than {MAX_EXPORT_RECORD_BYTES} bytes",
            row + 1
        )));
    }
    Ok(bytes)
}

struct LimitedBuffer {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}

impl LimitedBuffer {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            exceeded: false,
        }
    }
}

impl Write for LimitedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(new_len) = self.bytes.len().checked_add(bytes.len()) else {
            self.exceeded = true;
            return Err(io::Error::other("buffered export size overflow"));
        };
        if new_len > self.limit {
            self.exceeded = true;
            return Err(io::Error::other("buffered export limit exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{check_field_bytes, checked_record_bytes};
    use crate::csv::{MAX_EXPORT_RECORD_BYTES, MAX_FIELD_BYTES};

    #[test]
    fn export_field_and_record_limits_fail_without_allocating() {
        assert!(check_field_bytes(MAX_FIELD_BYTES, 0, 0).is_ok());
        assert_eq!(
            check_field_bytes(MAX_FIELD_BYTES + 1, 0, 0)
                .unwrap_err()
                .code,
            crate::error::codes::CSV_LIMIT
        );
        assert_eq!(
            checked_record_bytes(MAX_EXPORT_RECORD_BYTES, 1, 0)
                .unwrap_err()
                .code,
            crate::error::codes::CSV_LIMIT
        );
    }
}
