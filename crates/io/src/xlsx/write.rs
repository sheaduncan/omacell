//! Regenerate modeled OPC parts and re-emit preserved L3 bytes.

use std::collections::HashMap;
use std::io::{Cursor, Write};

use indexmap::IndexMap;
use omacell_core::addr::col_to_letters;
use omacell_core::error::CoreError;
use omacell_core::geometry::DEFAULT_COL_PX;
use omacell_core::sheet::{Sheet, SheetVisibility};
use omacell_core::storage::CellSlot;
use omacell_core::style::{BorderStyle, Color, Fill, Font, PatternType, Style, StyleId, Underline};
use omacell_core::tables::Table;
use omacell_core::value::Value;
use omacell_core::workbook::{CalcMode, DateSystem, Workbook};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use super::WorksheetExtras;
use super::opc::OpcPackage;
use super::{XlsxDocument, xml};
use crate::error;

const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_PKG: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const NS_CT: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const REL_OFFICE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
const REL_WS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";
const REL_SST: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings";
const REL_STYLES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
const REL_TABLE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/table";
const REL_COMMENTS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments";
const REL_HYPER: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
const REL_DRAWING: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing";
const REL_VML: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing";
const CT_WB: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml";
const CT_WS: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";
const CT_SST: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml";
const CT_STY: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml";
const CT_TBL: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml";
const CT_CMT: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml";

/// Encode `doc` as `.xlsx` bytes (modeled parts regenerated, L3 copied).
pub fn save_bytes(doc: &XlsxDocument) -> Result<Vec<u8>, CoreError> {
    encode(&doc.workbook, &doc.extras, Some(&doc.package))
}

/// Encode a workbook with no preserved package (new file).
pub fn save_workbook_bytes(wb: &Workbook) -> Result<Vec<u8>, CoreError> {
    encode(wb, &HashMap::new(), None)
}

pub(crate) fn encode(
    wb: &Workbook,
    extras: &HashMap<String, WorksheetExtras>,
    package: Option<&OpcPackage>,
) -> Result<Vec<u8>, CoreError> {
    let intern = wb.intern();
    let sheets: Vec<&Sheet> = wb.sheets().collect();
    if sheets.is_empty() {
        return Err(error::xlsx_write("workbook has no sheets"));
    }

    let mut sst: IndexMap<String, u32> = IndexMap::new();
    let mut fonts: Vec<Font> = vec![Font::default()];
    let mut fills: Vec<Fill> = vec![
        Fill::None,
        Fill::Pattern {
            pattern: PatternType::Gray125,
            fg: Color::Auto,
            bg: Color::Auto,
        },
    ];
    let mut borders = vec![omacell_core::style::Border::default()];
    let mut xfs: Vec<Style> = vec![Style::default()];
    let mut xf_index: HashMap<Style, usize> = HashMap::new();
    xf_index.insert(Style::default(), 0);

    for sheet in &sheets {
        for (_, _, slot) in sheet.store.iter() {
            if let Value::Text(id) = slot.value
                && let Some(text) = intern.strings.get(id)
                && !sst.contains_key(text)
            {
                let i = sst.len() as u32;
                sst.insert(text.to_string(), i);
            }
            if let Some(style) = intern.styles.get(slot.style) {
                xf_index.entry(style.clone()).or_insert_with(|| {
                    ensure_font(&mut fonts, &style.font);
                    ensure_fill(&mut fills, &style.fill);
                    if !borders.iter().any(|b| b == &style.border) {
                        borders.push(style.border);
                    }
                    let i = xfs.len();
                    xfs.push(style.clone());
                    i
                });
            }
        }
    }

    let mut parts: IndexMap<String, Vec<u8>> = IndexMap::new();
    let mut overrides: Vec<(String, String)> = Vec::new();

    parts.insert("xl/sharedStrings.xml".into(), sst_xml(&sst, intern));
    overrides.push(("/xl/sharedStrings.xml".into(), CT_SST.into()));
    parts.insert(
        "xl/styles.xml".into(),
        styles_xml(wb, &fonts, &fills, &borders, &xfs),
    );
    overrides.push(("/xl/styles.xml".into(), CT_STY.into()));

    let mut wb_rels: Vec<(String, String, String, bool)> = Vec::new();
    let mut rid = 1u32;
    let mut sheet_rids = Vec::new();
    for (i, sheet) in sheets.iter().enumerate() {
        let r = format!("rId{rid}");
        rid += 1;
        let target = format!("worksheets/sheet{}.xml", i + 1);
        wb_rels.push((r.clone(), REL_WS.into(), target.clone(), false));
        sheet_rids.push(r);
        let (sheet_xml, sheet_rels, extra_parts) = worksheet_xml(
            wb,
            sheet,
            extras.get(&sheet.name),
            &sst,
            &xf_index,
            intern,
            i,
            package,
        )?;
        parts.insert(format!("xl/{target}"), sheet_xml);
        overrides.push((format!("/xl/{target}"), CT_WS.into()));
        if !sheet_rels.is_empty() {
            parts.insert(
                format!("xl/worksheets/_rels/sheet{}.xml.rels", i + 1),
                rels_xml(&sheet_rels),
            );
        }
        for (name, bytes, ct) in extra_parts {
            overrides.push((format!("/{name}"), ct));
            parts.insert(name, bytes);
        }
    }
    let sst_rid = format!("rId{rid}");
    rid += 1;
    wb_rels.push((sst_rid, REL_SST.into(), "sharedStrings.xml".into(), false));
    let sty_rid = format!("rId{rid}");
    rid += 1;
    wb_rels.push((sty_rid, REL_STYLES.into(), "styles.xml".into(), false));

    if let Some(pkg) = package
        && let Ok(orig) = pkg.rels_for("xl/workbook.xml")
    {
        for rel in orig {
            if rel.rel_type == REL_WS || rel.rel_type == REL_SST || rel.rel_type == REL_STYLES {
                continue;
            }
            if is_rewritten(&rel.target) {
                continue;
            }
            let r = format!("rId{rid}");
            rid += 1;
            let target = if rel.external {
                rel.target.clone()
            } else {
                workbook_rel_target(&rel.target)
            };
            wb_rels.push((r, rel.rel_type, target, rel.external));
        }
    }

    parts.insert(
        "xl/workbook.xml".into(),
        workbook_xml(wb, intern, &sheets, &sheet_rids),
    );
    overrides.push(("/xl/workbook.xml".into(), CT_WB.into()));
    parts.insert("xl/_rels/workbook.xml.rels".into(), rels_xml(&wb_rels));

    for (name, bytes) in &wb.custom_parts {
        parts.insert(name.clone(), bytes.clone());
        overrides.push((format!("/{name}"), "application/json".into()));
    }

    if let Some(pkg) = package {
        for (name, part) in &pkg.parts {
            if is_rewritten(name) {
                continue;
            }
            if parts.contains_key(name) {
                continue;
            }
            parts.insert(name.clone(), part.bytes.clone());
            if let Some(ct) = &part.content_type {
                overrides.push((format!("/{name}"), ct.clone()));
            }
        }
    }

    parts.insert("[Content_Types].xml".into(), content_types_xml(&overrides));
    let mut pkg_rels = vec![(
        "rId1".into(),
        REL_OFFICE.into(),
        "xl/workbook.xml".into(),
        false,
    )];
    if let Some(pkg) = package {
        let mut n = 2u32;
        for rel in &pkg.package_rels {
            if rel.rel_type == REL_OFFICE {
                continue;
            }
            pkg_rels.push((
                format!("rId{n}"),
                rel.rel_type.clone(),
                rel.target.clone(),
                rel.external,
            ));
            n += 1;
        }
    }
    parts.insert("_rels/.rels".into(), rels_xml(&pkg_rels));

    zip_parts(&parts)
}

fn is_rewritten(name: &str) -> bool {
    let n = name.replace('\\', "/").to_ascii_lowercase();
    matches!(
        n.as_str(),
        "[content_types].xml"
            | "_rels/.rels"
            | "xl/workbook.xml"
            | "xl/_rels/workbook.xml.rels"
            | "xl/sharedstrings.xml"
            | "xl/styles.xml"
            | "xl/calcchain.xml"
    ) || n.starts_with("xl/worksheets/")
        || n.starts_with("xl/tables/")
        || n.starts_with("xl/comments")
        || n.starts_with("xl/omacell/")
}

fn workbook_rel_target(resolved: &str) -> String {
    resolved
        .strip_prefix("xl/")
        .or_else(|| resolved.strip_prefix("/xl/"))
        .unwrap_or(resolved)
        .to_string()
}

fn zip_parts(parts: &IndexMap<String, Vec<u8>>) -> Result<Vec<u8>, CoreError> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut z = ZipWriter::new(&mut buf);
        let opt = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let mut names: Vec<&String> = parts.keys().collect();
        names.sort_by(|a, b| part_order(a).cmp(&part_order(b)).then(a.cmp(b)));
        for name in names {
            let data = &parts[name];
            z.start_file(name, opt)
                .map_err(|e| error::xlsx_write(e.to_string()))?;
            z.write_all(data)
                .map_err(|e| error::xlsx_write(e.to_string()))?;
        }
        z.finish().map_err(|e| error::xlsx_write(e.to_string()))?;
    }
    Ok(buf.into_inner())
}

fn part_order(name: &str) -> u8 {
    match name {
        "[Content_Types].xml" => 0,
        "_rels/.rels" => 1,
        "xl/workbook.xml" => 2,
        "xl/_rels/workbook.xml.rels" => 3,
        _ if name.starts_with("xl/worksheets/") && !name.contains("_rels") => 4,
        _ if name.contains("/_rels/") => 5,
        "xl/sharedStrings.xml" => 6,
        "xl/styles.xml" => 7,
        _ => 8,
    }
}

fn workbook_xml(
    wb: &Workbook,
    intern: &omacell_core::intern::Interners,
    sheets: &[&Sheet],
    rids: &[String],
) -> Vec<u8> {
    let mut s = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="{NS}" xmlns:r="{NS_R}">"#
    );
    if wb.settings().date_system == DateSystem::Excel1904 {
        s.push_str(r#"<workbookPr date1904="1"/>"#);
    }
    let active = sheets
        .iter()
        .position(|sh| sh.id == wb.active_sheet())
        .unwrap_or(0);
    s.push_str(&format!(
        r#"<bookViews><workbookView activeTab="{active}"/></bookViews>"#
    ));
    s.push_str("<sheets>");
    for (i, sheet) in sheets.iter().enumerate() {
        let state = match sheet.visibility {
            SheetVisibility::Hidden => r#" state="hidden""#,
            SheetVisibility::VeryHidden => r#" state="veryHidden""#,
            SheetVisibility::Visible => "",
        };
        s.push_str(&format!(
            r#"<sheet name="{}" sheetId="{}" r:id="{}"{state}/>"#,
            xml::escape(&sheet.name),
            i + 1,
            rids[i]
        ));
    }
    s.push_str("</sheets>");
    let names: Vec<_> = wb.names().iter().collect();
    if !names.is_empty() {
        s.push_str("<definedNames>");
        for n in names {
            let local = match n.scope {
                omacell_core::names::NameScope::Workbook => String::new(),
                omacell_core::names::NameScope::Sheet(id) => sheets
                    .iter()
                    .position(|sh| sh.id == id)
                    .map(|i| format!(r#" localSheetId="{i}""#))
                    .unwrap_or_default(),
            };
            let text = match &n.referent {
                omacell_core::names::NameReferent::Range(r) => r.to_a1(),
                omacell_core::names::NameReferent::Formula(f) => f.clone(),
                omacell_core::names::NameReferent::Constant(v) => constant_name_text(intern, *v),
            };
            s.push_str(&format!(
                r#"<definedName name="{}"{local}>{}</definedName>"#,
                xml::escape(&n.name),
                xml::escape(&text)
            ));
        }
        s.push_str("</definedNames>");
    }
    match wb.settings().calc_mode {
        CalcMode::Manual => s.push_str(r#"<calcPr calcMode="manual"/>"#),
        CalcMode::AutomaticExceptTables => s.push_str(r#"<calcPr calcMode="autoNoTable"/>"#),
        CalcMode::Automatic => {}
    }
    s.push_str("</workbook>");
    s.into_bytes()
}

fn constant_name_text(intern: &omacell_core::intern::Interners, v: Value) -> String {
    match v {
        Value::Number(n) => n.to_string(),
        Value::Bool(true) => "TRUE".into(),
        Value::Bool(false) => "FALSE".into(),
        Value::Text(id) => intern
            .strings
            .get(id)
            .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
            .unwrap_or_default(),
        Value::Error(e) => e.as_str().to_string(),
        Value::Empty | Value::Array(_) => String::new(),
    }
}

fn sst_xml(sst: &IndexMap<String, u32>, intern: &omacell_core::intern::Interners) -> Vec<u8> {
    let _ = intern;
    let mut s = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><sst xmlns="{NS}" count="{}" uniqueCount="{}">"#,
        sst.len(),
        sst.len()
    );
    let mut items: Vec<(&String, &u32)> = sst.iter().collect();
    items.sort_by_key(|(_, i)| *i);
    for (text, _) in items {
        s.push_str("<si>");
        s.push_str(&t_elem(text));
        s.push_str("</si>");
    }
    s.push_str("</sst>");
    s.into_bytes()
}

fn t_elem(text: &str) -> String {
    if text.starts_with(' ') || text.ends_with(' ') || text.contains('\n') || text.contains('\t') {
        format!(r#"<t xml:space="preserve">{}</t>"#, xml::escape(text))
    } else {
        format!("<t>{}</t>", xml::escape(text))
    }
}

fn styles_xml(
    wb: &Workbook,
    fonts: &[Font],
    fills: &[Fill],
    borders: &[omacell_core::style::Border],
    xfs: &[Style],
) -> Vec<u8> {
    let mut numfmts: Vec<(u32, String)> = Vec::new();
    for xf in xfs {
        let id = xf.num_fmt.index();
        if id >= 164
            && let Some(code) = wb.num_fmt_code(xf.num_fmt)
            && !numfmts.iter().any(|(i, _)| *i == id)
        {
            numfmts.push((id, code.into_owned()));
        }
    }
    let mut s = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><styleSheet xmlns="{NS}">"#
    );
    if !numfmts.is_empty() {
        s.push_str(&format!(r#"<numFmts count="{}">"#, numfmts.len()));
        for (id, code) in &numfmts {
            s.push_str(&format!(
                r#"<numFmt numFmtId="{id}" formatCode="{}"/>"#,
                xml::escape(code)
            ));
        }
        s.push_str("</numFmts>");
    }
    s.push_str(&format!(r#"<fonts count="{}">"#, fonts.len()));
    for f in fonts {
        s.push_str(&font_xml(f));
    }
    s.push_str("</fonts>");
    s.push_str(&format!(r#"<fills count="{}">"#, fills.len()));
    for f in fills {
        s.push_str(&fill_xml(f));
    }
    s.push_str("</fills>");
    s.push_str(&format!(r#"<borders count="{}">"#, borders.len()));
    for b in borders {
        s.push_str(&border_xml(b));
    }
    s.push_str("</borders>");
    s.push_str(r#"<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>"#);
    s.push_str(&format!(r#"<cellXfs count="{}">"#, xfs.len()));
    for xf in xfs {
        let font_id = fonts.iter().position(|f| f == &xf.font).unwrap_or(0);
        let fill_id = fills.iter().position(|f| f == &xf.fill).unwrap_or(0);
        let border_id = borders.iter().position(|b| b == &xf.border).unwrap_or(0);
        let mut attrs = format!(
            r#" numFmtId="{}" fontId="{font_id}" fillId="{fill_id}" borderId="{border_id}" xfId="0""#,
            xf.num_fmt.index()
        );
        if xf.num_fmt.index() != 0 {
            attrs.push_str(r#" applyNumberFormat="1""#);
        }
        if font_id != 0 {
            attrs.push_str(r#" applyFont="1""#);
        }
        if fill_id != 0 {
            attrs.push_str(r#" applyFill="1""#);
        }
        if border_id != 0 {
            attrs.push_str(r#" applyBorder="1""#);
        }
        let align = alignment_xml(&xf.alignment);
        let prot = protection_xml(&xf.protection);
        if !align.is_empty() {
            attrs.push_str(r#" applyAlignment="1""#);
        }
        if !prot.is_empty() {
            attrs.push_str(r#" applyProtection="1""#);
        }
        if align.is_empty() && prot.is_empty() {
            s.push_str(&format!("<xf{attrs}/>"));
        } else {
            s.push_str(&format!("<xf{attrs}>{align}{prot}</xf>"));
        }
    }
    s.push_str("</cellXfs></styleSheet>");
    s.into_bytes()
}

fn alignment_xml(a: &omacell_core::style::Alignment) -> String {
    let def = omacell_core::style::Alignment::default();
    if a == &def {
        return String::new();
    }
    let mut attrs = String::new();
    if a.horizontal != def.horizontal {
        attrs.push_str(&format!(r#" horizontal="{}""#, h_align_name(a.horizontal)));
    }
    if a.vertical != def.vertical {
        attrs.push_str(&format!(r#" vertical="{}""#, v_align_name(a.vertical)));
    }
    if a.wrap {
        attrs.push_str(r#" wrapText="1""#);
    }
    if a.shrink {
        attrs.push_str(r#" shrinkToFit="1""#);
    }
    if a.indent != 0 {
        attrs.push_str(&format!(r#" indent="{}""#, a.indent));
    }
    if a.rotation != 0 {
        attrs.push_str(&format!(r#" textRotation="{}""#, a.rotation));
    }
    format!("<alignment{attrs}/>")
}

fn protection_xml(p: &omacell_core::style::Protection) -> String {
    let def = omacell_core::style::Protection::default();
    if p == &def {
        return String::new();
    }
    let mut attrs = String::new();
    if !p.locked {
        attrs.push_str(r#" locked="0""#);
    }
    if p.hidden {
        attrs.push_str(r#" hidden="1""#);
    }
    format!("<protection{attrs}/>")
}

fn h_align_name(h: omacell_core::style::HorizontalAlign) -> &'static str {
    use omacell_core::style::HorizontalAlign::*;
    match h {
        General => "general",
        Left => "left",
        Center => "center",
        Right => "right",
        Fill => "fill",
        Justify => "justify",
        CenterContinuous => "centerContinuous",
        Distributed => "distributed",
    }
}

fn v_align_name(v: omacell_core::style::VerticalAlign) -> &'static str {
    use omacell_core::style::VerticalAlign::*;
    match v {
        Top => "top",
        Center => "center",
        Bottom => "bottom",
        Justify => "justify",
        Distributed => "distributed",
    }
}

fn font_xml(f: &Font) -> String {
    let mut s = String::from("<font>");
    if f.bold {
        s.push_str("<b/>");
    }
    if f.italic {
        s.push_str("<i/>");
    }
    if f.strike {
        s.push_str("<strike/>");
    }
    match f.underline {
        Underline::None => {}
        Underline::Single => s.push_str("<u/>"),
        Underline::Double => s.push_str(r#"<u val="double"/>"#),
        Underline::SingleAccounting => s.push_str(r#"<u val="singleAccounting"/>"#),
        Underline::DoubleAccounting => s.push_str(r#"<u val="doubleAccounting"/>"#),
    }
    s.push_str(&format!(r#"<sz val="{}"/>"#, f.size_pt));
    s.push_str(&color_xml(&f.color));
    if !f.name.is_empty() {
        s.push_str(&format!(r#"<name val="{}"/>"#, xml::escape(&f.name)));
    }
    s.push_str("</font>");
    s
}

fn fill_xml(f: &Fill) -> String {
    match f {
        Fill::None => r#"<fill><patternFill patternType="none"/></fill>"#.into(),
        Fill::Solid { fg } => format!(
            r#"<fill><patternFill patternType="solid">{}</patternFill></fill>"#,
            color_tag("fgColor", fg)
        ),
        Fill::Pattern { pattern, fg, bg } => format!(
            r#"<fill><patternFill patternType="{}">{}{}</patternFill></fill>"#,
            pattern_name(*pattern),
            color_tag("fgColor", fg),
            color_tag("bgColor", bg)
        ),
        Fill::Gradient(g) => {
            let mut s = format!(r#"<fill><gradientFill degree="{}">"#, g.degree);
            for stop in &g.stops {
                s.push_str(&format!(
                    r#"<stop position="{}"><color rgb="{:08X}"/></stop>"#,
                    stop.position,
                    match stop.color {
                        Color::Rgb { argb } => argb,
                        _ => 0xFF00_0000,
                    }
                ));
            }
            s.push_str("</gradientFill></fill>");
            s
        }
    }
}

fn pattern_name(p: PatternType) -> &'static str {
    match p {
        PatternType::None => "none",
        PatternType::Solid => "solid",
        PatternType::MediumGray => "mediumGray",
        PatternType::DarkGray => "darkGray",
        PatternType::LightGray => "lightGray",
        PatternType::DarkHorizontal => "darkHorizontal",
        PatternType::DarkVertical => "darkVertical",
        PatternType::DarkDown => "darkDown",
        PatternType::DarkUp => "darkUp",
        PatternType::DarkGrid => "darkGrid",
        PatternType::DarkTrellis => "darkTrellis",
        PatternType::LightHorizontal => "lightHorizontal",
        PatternType::LightVertical => "lightVertical",
        PatternType::LightDown => "lightDown",
        PatternType::LightUp => "lightUp",
        PatternType::LightGrid => "lightGrid",
        PatternType::LightTrellis => "lightTrellis",
        PatternType::Gray125 => "gray125",
        PatternType::Gray0625 => "gray0625",
    }
}

fn border_xml(b: &omacell_core::style::Border) -> String {
    format!(
        "<border>{}{}{}{}</border>",
        border_side("left", &b.left),
        border_side("right", &b.right),
        border_side("top", &b.top),
        border_side("bottom", &b.bottom)
    )
}

fn border_side(name: &str, side: &omacell_core::style::BorderSide) -> String {
    let st = match side.style {
        BorderStyle::None => {
            return format!("<{name}/>");
        }
        BorderStyle::Thin => "thin",
        BorderStyle::Medium => "medium",
        BorderStyle::Dashed => "dashed",
        BorderStyle::Dotted => "dotted",
        BorderStyle::Thick => "thick",
        BorderStyle::Double => "double",
        BorderStyle::Hair => "hair",
        BorderStyle::MediumDashed => "mediumDashed",
        BorderStyle::DashDot => "dashDot",
        BorderStyle::MediumDashDot => "mediumDashDot",
        BorderStyle::DashDotDot => "dashDotDot",
        BorderStyle::MediumDashDotDot => "mediumDashDotDot",
        BorderStyle::SlantDashDot => "slantDashDot",
    };
    let color = color_tag("color", &side.color);
    if color.is_empty() {
        format!("<{name} style=\"{st}\"/>")
    } else {
        format!("<{name} style=\"{st}\">{color}</{name}>")
    }
}

fn color_xml(c: &Color) -> String {
    color_tag("color", c)
}

fn color_tag(tag: &str, c: &Color) -> String {
    match c {
        Color::Auto => String::new(),
        Color::Rgb { argb } => format!(r#"<{tag} rgb="{argb:08X}"/>"#),
        Color::Theme { theme, tint } if *tint == 0.0 => format!(r#"<{tag} theme="{theme}"/>"#),
        Color::Theme { theme, tint } => format!(r#"<{tag} theme="{theme}" tint="{tint}"/>"#),
        Color::Indexed { index } => format!(r#"<{tag} indexed="{index}"/>"#),
    }
}

fn ensure_font(fonts: &mut Vec<Font>, font: &Font) {
    if !fonts.iter().any(|f| f == font) {
        fonts.push(font.clone());
    }
}

fn ensure_fill(fills: &mut Vec<Fill>, fill: &Fill) {
    if !fills.iter().any(|f| f == fill) {
        fills.push(fill.clone());
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn worksheet_xml(
    wb: &Workbook,
    sheet: &Sheet,
    extras: Option<&WorksheetExtras>,
    sst: &IndexMap<String, u32>,
    xf_index: &HashMap<Style, usize>,
    intern: &omacell_core::intern::Interners,
    sheet_ord: usize,
    package: Option<&OpcPackage>,
) -> Result<
    (
        Vec<u8>,
        Vec<(String, String, String, bool)>,
        Vec<(String, Vec<u8>, String)>,
    ),
    CoreError,
> {
    let mut rels: Vec<(String, String, String, bool)> = Vec::new();
    let mut extra_parts = Vec::new();
    let mut rid = 1u32;
    let mut s = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="{NS}" xmlns:r="{NS_R}">"#
    );
    s.push_str(&sheet_views_xml(sheet));
    s.push_str(&cols_xml(sheet));
    s.push_str("<sheetData>");
    let mut cells_by_row: IndexMap<u32, Vec<(u16, CellSlot)>> = IndexMap::new();
    for (row, col, slot) in sheet.store.iter() {
        cells_by_row.entry(row).or_default().push((col, slot));
    }
    let hidden_rows: Vec<u32> = sheet.geometry.rows.iter_hidden().collect();
    let custom_rows: Vec<(u32, u32)> = sheet.geometry.rows.iter_custom().collect();
    let mut row_idxs: Vec<u32> = cells_by_row.keys().copied().collect();
    row_idxs.extend_from_slice(&hidden_rows);
    row_idxs.extend(custom_rows.iter().map(|(i, _)| *i));
    row_idxs.sort_unstable();
    row_idxs.dedup();
    for row in row_idxs {
        let r1 = row + 1;
        let mut attrs = format!(r#" r="{r1}""#);
        if hidden_rows.contains(&row) {
            attrs.push_str(r#" hidden="1""#);
        }
        if let Some((_, px)) = custom_rows.iter().find(|(i, _)| *i == row) {
            let ht = f64::from(*px) * 72.0 / 96.0;
            attrs.push_str(&format!(r#" ht="{ht}" customHeight="1""#));
        }
        let empty = Vec::new();
        let cells = cells_by_row.get(&row).unwrap_or(&empty);
        if cells.is_empty() {
            s.push_str(&format!("<row{attrs}/>"));
            continue;
        }
        s.push_str(&format!("<row{attrs}>"));
        for (col, slot) in cells {
            s.push_str(&cell_xml(row, *col, slot, sst, xf_index, intern)?);
        }
        s.push_str("</row>");
    }
    s.push_str("</sheetData>");
    if sheet.protection.enabled {
        s.push_str(r#"<sheetProtection sheet="1"/>"#);
    }
    if let Some(ex) = extras
        && let Some(af) = &ex.autofilter
    {
        s.push_str(&format!(r#"<autoFilter ref="{}"/>"#, xml::escape(af)));
    }
    if !sheet.merges.is_empty() {
        s.push_str(&format!(r#"<mergeCells count="{}">"#, sheet.merges.len()));
        for m in &sheet.merges {
            s.push_str(&format!(
                r#"<mergeCell ref="{}"/>"#,
                xml::escape(&m.to_a1())
            ));
        }
        s.push_str("</mergeCells>");
    }
    if let Some(ex) = extras {
        for blob in &ex.conditional_formatting_xml {
            s.push_str(std::str::from_utf8(blob).unwrap_or(""));
        }
        for blob in &ex.data_validations_xml {
            s.push_str(std::str::from_utf8(blob).unwrap_or(""));
        }
    }
    if !sheet.hyperlinks.is_empty() {
        s.push_str("<hyperlinks>");
        let mut hrefs: Vec<_> = sheet.hyperlinks.iter().collect();
        hrefs.sort_by_key(|((r, c), _)| (*r, *c));
        for ((row, col), link) in hrefs {
            let addr = format!(
                "{}{}",
                col_to_letters(*col).unwrap_or_else(|_| "A".into()),
                row + 1
            );
            let id = format!("rId{rid}");
            rid += 1;
            rels.push((id.clone(), REL_HYPER.into(), link.target.clone(), true));
            let display = link
                .display
                .as_ref()
                .map(|d| format!(r#" display="{}""#, xml::escape(d)))
                .unwrap_or_default();
            s.push_str(&format!(
                r#"<hyperlink ref="{addr}" r:id="{id}"{display}/>"#
            ));
        }
        s.push_str("</hyperlinks>");
    }
    if let Some(ex) = extras {
        for blob in &ex.print_xml {
            s.push_str(std::str::from_utf8(blob).unwrap_or(""));
        }
    }
    let mut drawing_xml = String::new();
    let mut vml_xml = String::new();
    if let Some(pkg) = package
        && let Ok(orig) = original_sheet_rels(pkg, sheet_ord)
    {
        for rel in orig {
            if rel.rel_type == REL_HYPER
                || rel.rel_type == REL_TABLE
                || rel.rel_type == REL_COMMENTS
            {
                continue;
            }
            let id = format!("rId{rid}");
            rid += 1;
            let target = if rel.external {
                rel.target.clone()
            } else {
                sheet_rel_target(&rel.target)
            };
            if rel.rel_type == REL_DRAWING {
                drawing_xml = format!(r#"<drawing r:id="{id}"/>"#);
            } else if rel.rel_type == REL_VML {
                vml_xml = format!(r#"<legacyDrawing r:id="{id}"/>"#);
            }
            rels.push((id, rel.rel_type, target, rel.external));
        }
    }
    s.push_str(&drawing_xml);
    s.push_str(&vml_xml);
    let tables: Vec<&Table> = wb.tables().iter().filter(|t| t.sheet == sheet.id).collect();
    if !tables.is_empty() {
        s.push_str(&format!(r#"<tableParts count="{}">"#, tables.len()));
        for (i, table) in tables.iter().enumerate() {
            let id = format!("rId{rid}");
            rid += 1;
            let tname = format!("xl/tables/table{}{}.xml", sheet_ord + 1, i + 1);
            rels.push((
                id.clone(),
                REL_TABLE.into(),
                format!("../tables/table{}{}.xml", sheet_ord + 1, i + 1),
                false,
            ));
            extra_parts.push((tname, table_xml(table), CT_TBL.into()));
            s.push_str(&format!(r#"<tablePart r:id="{id}"/>"#));
        }
        s.push_str("</tableParts>");
    }
    if let Some(ex) = extras {
        for blob in &ex.sparkline_xml {
            s.push_str(std::str::from_utf8(blob).unwrap_or(""));
        }
    }
    if !sheet.notes.is_empty() {
        let id = format!("rId{rid}");
        let cname = format!("xl/comments{}.xml", sheet_ord + 1);
        rels.push((
            id,
            REL_COMMENTS.into(),
            format!("../comments{}.xml", sheet_ord + 1),
            false,
        ));
        extra_parts.push((cname, comments_xml(sheet), CT_CMT.into()));
    }
    let _ = rid;
    s.push_str("</worksheet>");
    Ok((s.into_bytes(), rels, extra_parts))
}

fn original_sheet_rels(
    pkg: &OpcPackage,
    sheet_ord: usize,
) -> Result<Vec<super::opc::Relationship>, CoreError> {
    let wb = pkg.workbook_part()?;
    let rels = pkg.rels_for(&wb.name)?;
    let sheets: Vec<_> = rels.into_iter().filter(|r| r.rel_type == REL_WS).collect();
    let Some(rel) = sheets.get(sheet_ord) else {
        return Ok(Vec::new());
    };
    pkg.rels_for(&rel.target)
}

fn sheet_rel_target(resolved: &str) -> String {
    let t = resolved.trim_start_matches('/');
    if let Some(rest) = t.strip_prefix("xl/") {
        format!("../{rest}")
    } else {
        t.to_string()
    }
}

fn sheet_views_xml(sheet: &Sheet) -> String {
    let v = &sheet.view;
    let zoom = if (v.zoom - 1.0).abs() < f64::EPSILON {
        String::new()
    } else {
        format!(r#" zoomScale="{}""#, (v.zoom * 100.0).round())
    };
    let grid = if v.gridlines {
        String::new()
    } else {
        r#" showGridLines="0""#.into()
    };
    let formulas = if v.show_formulas {
        r#" showFormulas="1""#
    } else {
        ""
    };
    let mut pane = String::new();
    if v.freeze.rows > 0 || v.freeze.cols > 0 {
        let top_left = format!(
            "{}{}",
            col_to_letters(v.freeze.cols).unwrap_or_else(|_| "A".into()),
            v.freeze.rows + 1
        );
        pane = format!(
            r#"<pane xSplit="{}" ySplit="{}" topLeftCell="{top_left}" activePane="bottomRight" state="frozen"/>"#,
            v.freeze.cols, v.freeze.rows
        );
    } else if let Some(split) = v.split {
        pane = format!(
            r#"<pane xSplit="{}" ySplit="{}" state="split"/>"#,
            split.x_px, split.y_px
        );
    }
    format!(
        r#"<sheetViews><sheetView workbookViewId="0"{zoom}{grid}{formulas}>{pane}</sheetView></sheetViews>"#
    )
}

fn cols_xml(sheet: &Sheet) -> String {
    let hidden: Vec<u32> = sheet.geometry.cols.iter_hidden().collect();
    let custom: Vec<(u32, u32)> = sheet.geometry.cols.iter_custom().collect();
    if hidden.is_empty() && custom.is_empty() {
        return String::new();
    }
    let mut idxs: Vec<u32> = hidden
        .iter()
        .copied()
        .chain(custom.iter().map(|(i, _)| *i))
        .collect();
    idxs.sort_unstable();
    idxs.dedup();
    let mut s = String::from("<cols>");
    for i in idxs {
        let min = i + 1;
        let hidden_attr = if hidden.contains(&i) {
            r#" hidden="1""#
        } else {
            ""
        };
        let width = custom
            .iter()
            .find(|(j, _)| *j == i)
            .map(|(_, px)| f64::from(*px) * 8.43 / f64::from(DEFAULT_COL_PX))
            .unwrap_or(8.43);
        s.push_str(&format!(
            r#"<col min="{min}" max="{min}" width="{width}" customWidth="1"{hidden_attr}/>"#
        ));
    }
    s.push_str("</cols>");
    s
}

fn cell_xml(
    row: u32,
    col: u16,
    slot: &CellSlot,
    sst: &IndexMap<String, u32>,
    xf_index: &HashMap<Style, usize>,
    intern: &omacell_core::intern::Interners,
) -> Result<String, CoreError> {
    let addr = format!(
        "{}{}",
        col_to_letters(col).map_err(|e| error::xlsx_write(e.to_string()))?,
        row + 1
    );
    let mut attrs = format!(r#" r="{addr}""#);
    if slot.style != StyleId::DEFAULT
        && let Some(style) = intern.styles.get(slot.style)
        && let Some(i) = xf_index.get(style)
        && *i > 0
    {
        attrs.push_str(&format!(r#" s="{i}""#));
    }
    let mut inner = String::new();
    if let Some(fid) = slot.formula
        && let Some(src) = intern.formulas.get(fid)
    {
        let body = src.strip_prefix('=').unwrap_or(src);
        inner.push_str(&format!("<f>{}</f>", xml::escape(body)));
    }
    match slot.value {
        Value::Number(n) => {
            inner.push_str(&format!("<v>{n}</v>"));
        }
        Value::Bool(b) => {
            attrs.push_str(r#" t="b""#);
            inner.push_str(&format!("<v>{}</v>", if b { "1" } else { "0" }));
        }
        Value::Error(e) => {
            attrs.push_str(r#" t="e""#);
            inner.push_str(&format!("<v>{}</v>", xml::escape(e.as_str())));
        }
        Value::Text(id) => {
            if let Some(text) = intern.strings.get(id) {
                if let Some(idx) = sst.get(text) {
                    attrs.push_str(r#" t="s""#);
                    inner.push_str(&format!("<v>{idx}</v>"));
                } else {
                    attrs.push_str(r#" t="inlineStr""#);
                    inner.push_str(&format!("<is>{}</is>", t_elem(text)));
                }
            }
        }
        Value::Empty | Value::Array(_) => {}
    }
    if inner.is_empty() {
        Ok(format!("<c{attrs}/>"))
    } else {
        Ok(format!("<c{attrs}>{inner}</c>"))
    }
}

fn table_xml(table: &Table) -> Vec<u8> {
    let start = format!(
        "{}{}",
        col_to_letters(table.start_col).unwrap_or_else(|_| "A".into()),
        table.start_row + 1
    );
    let end = format!(
        "{}{}",
        col_to_letters(table.end_col).unwrap_or_else(|_| "A".into()),
        table.end_row + 1
    );
    let header = if table.has_header { 1 } else { 0 };
    let totals = if table.has_totals { 1 } else { 0 };
    let mut cols = String::new();
    for (i, c) in table.columns.iter().enumerate() {
        cols.push_str(&format!(
            r#"<tableColumn id="{}" name="{}"/>"#,
            i + 1,
            xml::escape(&c.name)
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><table xmlns="{NS}" id="1" name="{}" displayName="{}" ref="{start}:{end}" headerRowCount="{header}" totalsRowCount="{totals}"><tableColumns count="{}">{cols}</tableColumns></table>"#,
        xml::escape(&table.name),
        xml::escape(&table.name),
        table.columns.len()
    )
    .into_bytes()
}

fn comments_xml(sheet: &Sheet) -> Vec<u8> {
    let mut authors: Vec<String> = Vec::new();
    let mut notes: Vec<_> = sheet.notes.iter().collect();
    notes.sort_by_key(|((r, c), _)| (*r, *c));
    for (_, n) in &notes {
        let a = n.author.clone().unwrap_or_default();
        if !authors.iter().any(|x| x == &a) {
            authors.push(a);
        }
    }
    let mut s = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><comments xmlns="{NS}"><authors>"#
    );
    for a in &authors {
        s.push_str(&format!("<author>{}</author>", xml::escape(a)));
    }
    s.push_str("</authors><commentList>");
    for ((row, col), n) in notes {
        let addr = format!(
            "{}{}",
            col_to_letters(*col).unwrap_or_else(|_| "A".into()),
            row + 1
        );
        let aid = n
            .author
            .as_ref()
            .and_then(|a| authors.iter().position(|x| x == a))
            .unwrap_or(0);
        s.push_str(&format!(
            r#"<comment ref="{addr}" authorId="{aid}"><text>{}</text></comment>"#,
            t_elem(&n.text)
        ));
    }
    s.push_str("</commentList></comments>");
    s.into_bytes()
}

fn rels_xml(rels: &[(String, String, String, bool)]) -> Vec<u8> {
    let mut s = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="{NS_PKG}">"#
    );
    for (id, ty, target, external) in rels {
        let mode = if *external {
            r#" TargetMode="External""#
        } else {
            ""
        };
        s.push_str(&format!(
            r#"<Relationship Id="{id}" Type="{ty}" Target="{}"{mode}/>"#,
            xml::escape(target)
        ));
    }
    s.push_str("</Relationships>");
    s.into_bytes()
}

fn content_types_xml(overrides: &[(String, String)]) -> Vec<u8> {
    let mut s = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="{NS_CT}"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="json" ContentType="application/json"/><Default Extension="bin" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.printerSettings"/><Default Extension="vml" ContentType="application/vnd.openxmlformats-officedocument.vmlDrawing"/>"#
    );
    let mut seen = std::collections::HashSet::new();
    for (part, ct) in overrides {
        let key = part.to_ascii_lowercase();
        if seen.insert(key) {
            s.push_str(&format!(
                r#"<Override PartName="{}" ContentType="{}"/>"#,
                xml::escape(part),
                xml::escape(ct)
            ));
        }
    }
    s.push_str("</Types>");
    s.into_bytes()
}
