//! Basic ODS writer (values, formulas, merges, names, bold/fill).

use std::io::{Cursor, Write};

use omacell_core::addr::col_to_letters;
use omacell_core::error::CoreError;
use omacell_core::names::NameReferent;
use omacell_core::style::{Color, Fill};
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::error;
use crate::xlsx::opc::MAX_PACKAGE_BYTES;
use crate::xlsx::xml::escape;

/// Encode a workbook as ODS bytes.
pub fn save_bytes(wb: &Workbook) -> Result<Vec<u8>, CoreError> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut z = ZipWriter::new(&mut buf);
        let stored =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file("mimetype", stored)
            .map_err(|e| error::ods_format(e.to_string()))?;
        z.write_all(b"application/vnd.oasis.opendocument.spreadsheet")
            .map_err(|e| error::ods_format(e.to_string()))?;
        let deflated =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        z.start_file("META-INF/manifest.xml", deflated)
            .map_err(|e| error::ods_format(e.to_string()))?;
        z.write_all(manifest().as_bytes())
            .map_err(|e| error::ods_format(e.to_string()))?;
        z.start_file("content.xml", deflated)
            .map_err(|e| error::ods_format(e.to_string()))?;
        z.write_all(content_xml(wb)?.as_bytes())
            .map_err(|e| error::ods_format(e.to_string()))?;
        z.finish().map_err(|e| error::ods_format(e.to_string()))?;
    }
    let bytes = buf.into_inner();
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(error::xlsx_limit("ODS output exceeds the package cap"));
    }
    Ok(bytes)
}

fn manifest() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3">
 <manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/>
 <manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
</manifest:manifest>
"#
    .into()
}

fn content_xml(wb: &Workbook) -> Result<String, CoreError> {
    let mut out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" office:version="1.3">
<office:automatic-styles>
"#,
    );
    let mut style_names: Vec<(String, StyleBits)> = Vec::new();
    for sheet in wb.sheets() {
        if let Some(used) = sheet.used_range() {
            for row in used.min_row..=used.max_row {
                for col in used.min_col..=used.max_col {
                    if let Some(slot) = wb.get(sheet.id, row, col).ok().flatten() {
                        let style = wb
                            .intern()
                            .styles
                            .get(slot.style)
                            .cloned()
                            .unwrap_or_default();
                        let bits = StyleBits::from_style(&style);
                        if !bits.is_default() {
                            let name = format!("ce{}", style_names.len() + 1);
                            if !style_names.iter().any(|(_, b)| *b == bits) {
                                out.push_str(&bits.xml(&name));
                                style_names.push((name, bits));
                            }
                        }
                    }
                }
            }
        }
    }
    out.push_str("</office:automatic-styles><office:body><office:spreadsheet>");
    for sheet in wb.sheets() {
        out.push_str(&format!(
            r#"<table:table table:name="{}">"#,
            escape(&sheet.name)
        ));
        if let Some(used) = sheet.used_range() {
            for row in 0..=used.max_row {
                out.push_str("<table:table-row>");
                for col in 0..=used.max_col {
                    match wb.get(sheet.id, row, col).ok().flatten() {
                        Some(slot) => {
                            let style = wb
                                .intern()
                                .styles
                                .get(slot.style)
                                .cloned()
                                .unwrap_or_default();
                            let bits = StyleBits::from_style(&style);
                            let style_attr = style_names
                                .iter()
                                .find(|(_, b)| *b == bits)
                                .map(|(n, _)| format!(r#" table:style-name="{n}""#))
                                .unwrap_or_default();
                            let merge = sheet
                                .merges
                                .iter()
                                .find(|m| m.start.row == row && m.start.col == col);
                            let span = merge
                                .map(|m| {
                                    let cols = m.end.col.saturating_sub(m.start.col) + 1;
                                    let rows = m.end.row.saturating_sub(m.start.row) + 1;
                                    format!(
                                        r#" table:number-columns-spanned="{cols}" table:number-rows-spanned="{rows}""#
                                    )
                                })
                                .unwrap_or_default();
                            if let Some(fid) = slot.formula {
                                let src = wb.intern().formulas.get(fid).unwrap_or("");
                                let of = excel_formula_to_ods(src);
                                out.push_str(&format!(
                                    r#"<table:table-cell table:formula="{}"{style_attr}{span}/>"#,
                                    escape(&of)
                                ));
                            } else {
                                match slot.value {
                                    Value::Number(n) => {
                                        out.push_str(&format!(
                                            r#"<table:table-cell office:value-type="float" office:value="{n}"{style_attr}{span}><text:p>{n}</text:p></table:table-cell>"#
                                        ));
                                    }
                                    Value::Bool(b) => {
                                        let v = if b { "true" } else { "false" };
                                        out.push_str(&format!(
                                            r#"<table:table-cell office:value-type="boolean" office:boolean-value="{v}"{style_attr}{span}><text:p>{v}</text:p></table:table-cell>"#
                                        ));
                                    }
                                    Value::Text(id) => {
                                        let t = wb.intern().strings.get(id).unwrap_or("");
                                        out.push_str(&format!(
                                            r#"<table:table-cell office:value-type="string"{style_attr}{span}><text:p>{}</text:p></table:table-cell>"#,
                                            escape(t)
                                        ));
                                    }
                                    Value::Error(kind) => {
                                        out.push_str(&format!(
                                            r#"<table:table-cell office:value-type="string"{style_attr}{span}><text:p>{}</text:p></table:table-cell>"#,
                                            escape(kind.as_str())
                                        ));
                                    }
                                    Value::Empty | Value::Array(_) => {
                                        out.push_str("<table:table-cell/>");
                                    }
                                }
                            }
                        }
                        None => out.push_str("<table:table-cell/>"),
                    }
                }
                out.push_str("</table:table-row>");
            }
        }
        out.push_str("</table:table>");
    }
    out.push_str("<table:named-expressions>");
    for name in wb.names().iter() {
        if let NameReferent::Range(range) = &name.referent {
            let sheet_name = wb
                .sheet(range.start.sheet.unwrap_or_else(|| wb.active_sheet()))
                .map(|s| s.name.as_str())
                .unwrap_or("Sheet1");
            let a = format!(
                "{}{}",
                col_to_letters(range.start.col).unwrap_or_else(|_| "A".into()),
                range.start.row + 1
            );
            let b = format!(
                "{}{}",
                col_to_letters(range.end.col).unwrap_or_else(|_| "A".into()),
                range.end.row + 1
            );
            let addr = if a == b {
                format!("${sheet_name}.${a}")
            } else {
                format!("${sheet_name}.${a}:${b}")
            };
            out.push_str(&format!(
                r#"<table:named-range table:name="{}" table:cell-range-address="{}"/>"#,
                escape(&name.name),
                escape(&addr)
            ));
        }
    }
    out.push_str(
        "</table:named-expressions></office:spreadsheet></office:body></office:document-content>",
    );
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StyleBits {
    bold: bool,
    italic: bool,
    color: Option<u32>,
    fill: Option<u32>,
}

impl StyleBits {
    fn from_style(style: &omacell_core::style::Style) -> Self {
        Self {
            bold: style.font.bold,
            italic: style.font.italic,
            color: rgb(style.font.color),
            fill: match style.fill {
                Fill::Solid { fg } => rgb(fg),
                _ => None,
            },
        }
    }

    fn is_default(self) -> bool {
        !self.bold && !self.italic && self.color.is_none() && self.fill.is_none()
    }

    fn xml(self, name: &str) -> String {
        let mut text = String::new();
        if self.bold {
            text.push_str(r#" fo:font-weight="bold""#);
        }
        if self.italic {
            text.push_str(r#" fo:font-style="italic""#);
        }
        if let Some(c) = self.color {
            let hex = format!("{:06X}", c & 0x00FF_FFFF);
            text.push_str(" fo:color=\"#");
            text.push_str(&hex);
            text.push('"');
        }
        let mut cell = String::new();
        if let Some(c) = self.fill {
            let hex = format!("{:06X}", c & 0x00FF_FFFF);
            cell.push_str(" fo:background-color=\"#");
            cell.push_str(&hex);
            cell.push('"');
        }
        format!(
            r#"<style:style style:name="{name}" style:family="table-cell"><style:text-properties{text}/><style:table-cell-properties{cell}/></style:style>"#
        )
    }
}

fn rgb(color: Color) -> Option<u32> {
    match color {
        Color::Rgb { argb } => Some(argb),
        _ => None,
    }
}

fn excel_formula_to_ods(src: &str) -> String {
    let body = src.trim().trim_start_matches('=');
    format!("of:={body}")
}
