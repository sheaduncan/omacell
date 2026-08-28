//! Delimited export.

use std::io::Write;

use omacell_core::addr::{ParsedRef, RangeRef, RefKind, SheetId, parse_a1};
use omacell_core::error::CoreError;
use omacell_core::numfmt::{FormatOptions, FormatValue, format_with};
use omacell_core::storage::UsedRange;
use omacell_core::style::NumFmtId;
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;

use super::encode::encode_all;
use super::plan::{ExportPlan, LineEnding, Quoting, TextEncoding, ValueMode};
use crate::error;

/// Export `wb` to a UTF-8-decoded then encoded buffer.
pub fn export(wb: &Workbook, plan: &ExportPlan) -> Result<Vec<u8>, CoreError> {
    let mut buf = Vec::new();
    export_write(wb, plan, &mut buf)?;
    Ok(buf)
}

/// Export `wb` to `dest`.
pub fn export_write<W: Write>(
    wb: &Workbook,
    plan: &ExportPlan,
    mut dest: W,
) -> Result<(), CoreError> {
    plan.validate()?;
    let utf8 = export_utf8(wb, plan)?;
    let encoded = match plan.encoding {
        TextEncoding::Utf8 => {
            let mut out = Vec::with_capacity(utf8.len() + 3);
            if plan.bom {
                out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
            }
            out.extend_from_slice(&utf8);
            out
        }
        other => encode_all(
            std::str::from_utf8(&utf8)
                .map_err(|_| error::export("internal UTF-8 export failed"))?,
            other,
            plan.bom || matches!(other, TextEncoding::Utf16Le | TextEncoding::Utf16Be),
        )?,
    };
    dest.write_all(&encoded)
        .map_err(|e| error::export(e.to_string()))?;
    Ok(())
}

fn export_utf8(wb: &Workbook, plan: &ExportPlan) -> Result<Vec<u8>, CoreError> {
    let sheet = resolve_sheet(wb, plan.sheet.as_deref())?;
    let bounds = export_bounds(wb, sheet, plan.range.as_deref())?;
    let mut wtr = ::csv::WriterBuilder::new()
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
        })
        .from_writer(Vec::<u8>::new());

    if let Some((r0, c0, r1, c1)) = bounds {
        for row in r0..=r1 {
            let mut rec = Vec::with_capacity((u32::from(c1 - c0) + 1) as usize);
            for col in c0..=c1 {
                rec.push(cell_text(wb, sheet, row, col, plan)?);
            }
            if plan.quoting == Quoting::Never
                && let Some((col, field)) = rec.iter().enumerate().find(|(_, field)| {
                    field.contains(plan.delimiter)
                        || field.contains(plan.quote)
                        || field.contains(['\r', '\n'])
                })
            {
                return Err(error::export(format!(
                    "cell at row {}, column {} requires quoting: {field:?}",
                    row + 1,
                    u32::from(c0) + col as u32 + 1
                )));
            }
            wtr.write_record(&rec)
                .map_err(|e| error::export(e.to_string()))?;
        }
    }
    wtr.into_inner().map_err(|e| error::export(e.to_string()))
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
        Value::Text(id) => Ok(wb.intern().strings.get(id).unwrap_or("").to_string()),
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
            Ok(format_with(FormatValue::Number(n), &code, &opts).text)
        }
    }
}
