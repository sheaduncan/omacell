//! Load an OPC package into a [`Workbook`].

use std::collections::HashMap;

use omacell_core::addr::{CellRef, RangeRef, RefKind, parse_a1, parse_a1_cell};
use omacell_core::error::{CoreError, ErrorKind};
use omacell_core::formula::{copy_delta, parse, print};
use omacell_core::intern::RichTextRun;
use omacell_core::names::{DefinedName, NameReferent, NameScope};
use omacell_core::sheet::{
    FreezePanes, Hyperlink, Note, ProtectionState, SheetVisibility, SplitView,
};
use omacell_core::storage::{CellFlags, CellSlot};
use omacell_core::style::{
    Alignment, Border, BorderSide, BorderStyle, Color, Fill, Font, GradientFill, GradientKind,
    GradientStop, HorizontalAlign, NumFmtId, PatternType, Protection, Style, Underline,
    VerticalAlign,
};
use omacell_core::tables::{Table, TableColumn};
use omacell_core::value::Value;
use omacell_core::workbook::{CalcMode, DateSystem, Workbook};

use super::XlsxDocument;
use super::opc::{OpcPackage, Relationship, open_package};
use super::warnings::FileWarnings;
use super::xml::{XmlEvent, XmlReader, attr};
use crate::error;

const REL_WS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";
const REL_SST: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings";
const REL_STYLES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
const REL_THEME: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme";
const REL_TABLE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/table";
const REL_COMMENTS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments";
const REL_HYPER: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";

/// Unmodeled worksheet fragments for WP-10.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorksheetExtras {
    /// AutoFilter `ref`.
    pub autofilter: Option<String>,
    /// Raw `pageSetup` / `pageMargins` / `printOptions` / `headerFooter` XML.
    pub print_xml: Vec<String>,
    /// Raw `conditionalFormatting` XML blobs.
    pub conditional_formatting_xml: Vec<String>,
    /// Raw `dataValidations` / `extLst` validation XML.
    pub data_validations_xml: Vec<String>,
    /// Sparkline groups (`x14`).
    pub sparkline_xml: Vec<String>,
}

pub(crate) fn load(bytes: &[u8]) -> Result<XlsxDocument, CoreError> {
    let package = open_package(bytes)?;
    let mut warnings = FileWarnings::new();
    let mut wb = Workbook::new();
    wb.undo_log_mut().set_enabled(false);

    let wb_part = package.workbook_part()?;
    let wb_name = wb_part.name.clone();
    let wb_bytes = wb_part.bytes.clone();
    let wb_rels = package.rels_for(&wb_name)?;
    let theme = load_theme(&package, &wb_rels, &mut warnings);
    let sst = load_sst(&package, &wb_rels, &mut warnings)?;
    let styles = load_styles(&package, &wb_rels, &theme, &mut warnings)?;
    let sheets_meta = parse_workbook_xml(&wb_bytes, &mut wb, &mut warnings)?;

    let first_id = wb.active_sheet();
    let mut extras: HashMap<String, WorksheetExtras> = HashMap::new();

    for (i, meta) in sheets_meta.iter().enumerate() {
        let rel = wb_rels.iter().find(|r| r.id == meta.rid);
        let Some(rel) = rel else {
            warnings.push(
                "xlsx.part",
                format!("sheet {} has no relationship {}", meta.name, meta.rid),
                Some(wb_name.clone()),
            );
            continue;
        };
        if rel.rel_type != REL_WS {
            warnings.push(
                "xlsx.part",
                format!("sheet {} target is not a worksheet", meta.name),
                Some(rel.target.clone()),
            );
            continue;
        }
        let id = if i == 0 {
            wb.rename_sheet(first_id, &meta.name)?;
            first_id
        } else {
            wb.add_sheet(&meta.name)?
        };
        if meta.hidden {
            let _ = wb.set_visibility(id, SheetVisibility::Hidden);
        }
        if meta.very_hidden {
            let _ = wb.set_visibility(id, SheetVisibility::VeryHidden);
        }
        let sheet_rels = package.rels_for(&rel.target).unwrap_or_default();
        let extra = load_sheet(
            &mut wb,
            id,
            &package,
            &rel.target,
            &sheet_rels,
            &sst,
            &styles,
            &mut warnings,
        )?;
        extras.insert(meta.name.clone(), extra);
        load_tables(&mut wb, id, &package, &sheet_rels, &mut warnings)?;
        load_comments(&mut wb, id, &package, &sheet_rels, &mut warnings)?;
    }

    load_omacell_parts(&mut wb, &package);

    let mut doc = XlsxDocument {
        workbook: wb,
        warnings,
        package,
        extras,
    };
    doc.workbook.undo_log_mut().set_enabled(true);
    Ok(doc)
}

struct SheetMeta {
    name: String,
    rid: String,
    hidden: bool,
    very_hidden: bool,
}

fn parse_workbook_xml(
    bytes: &[u8],
    wb: &mut Workbook,
    warnings: &mut FileWarnings,
) -> Result<Vec<SheetMeta>, CoreError> {
    let mut r = XmlReader::new(bytes);
    let mut sheets = Vec::new();
    let mut in_sheets = false;
    let mut in_names = false;
    let mut name_attrs: Vec<(String, String)> = Vec::new();
    let mut name_text = String::new();
    while let Some(ev) = r.next()? {
        match ev {
            XmlEvent::Start { name, attrs: _ } if name == "sheets" => in_sheets = true,
            XmlEvent::End { name } if name == "sheets" => in_sheets = false,
            XmlEvent::Start { name, attrs } | XmlEvent::Empty { name, attrs }
                if in_sheets && name == "sheet" =>
            {
                let nm = attr(&attrs, "name").unwrap_or("Sheet").to_string();
                let rid = attr(&attrs, "id").unwrap_or("").to_string();
                let state = attr(&attrs, "state").unwrap_or("");
                sheets.push(SheetMeta {
                    name: nm,
                    rid,
                    hidden: state.eq_ignore_ascii_case("hidden"),
                    very_hidden: state.eq_ignore_ascii_case("veryHidden"),
                });
            }
            XmlEvent::Empty { name, attrs } if name == "workbookPr" => {
                if attr(&attrs, "date1904").is_some_and(truthy) {
                    wb.settings_mut().date_system = DateSystem::Excel1904;
                }
            }
            XmlEvent::Start { name, attrs } if name == "workbookPr" => {
                if attr(&attrs, "date1904").is_some_and(truthy) {
                    wb.settings_mut().date_system = DateSystem::Excel1904;
                }
            }
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if name == "calcPr" =>
            {
                match attr(&attrs, "calcMode").unwrap_or("auto") {
                    "manual" => wb.settings_mut().calc_mode = CalcMode::Manual,
                    "autoNoTable" => wb.settings_mut().calc_mode = CalcMode::AutomaticExceptTables,
                    _ => wb.settings_mut().calc_mode = CalcMode::Automatic,
                }
            }
            XmlEvent::Start { name, .. } if name == "definedNames" => in_names = true,
            XmlEvent::End { name } if name == "definedNames" => in_names = false,
            XmlEvent::Start { name, attrs } if in_names && name == "definedName" => {
                name_attrs = attrs;
                name_text.clear();
            }
            XmlEvent::Text(t) if in_names => name_text.push_str(&t),
            XmlEvent::End { name } if in_names && name == "definedName" => {
                let nm = attr(&name_attrs, "name").unwrap_or("").to_string();
                if !nm.is_empty() {
                    let local = attr(&name_attrs, "localSheetId");
                    let scope = if let Some(idx) = local.and_then(|s| s.parse::<usize>().ok()) {
                        sheets
                            .get(idx)
                            .and_then(|m| wb.sheet_by_name(&m.name).map(|s| s.id))
                            .map(NameScope::Sheet)
                            .unwrap_or(NameScope::Workbook)
                    } else {
                        NameScope::Workbook
                    };
                    let referent = parse_name_ref(name_text.trim());
                    if let Err(e) = wb.define_name(DefinedName {
                        name: nm,
                        scope,
                        referent,
                        comment: None,
                    }) {
                        warnings.push("xlsx.name", e.message, Some("xl/workbook.xml".into()));
                    }
                }
                name_text.clear();
            }
            _ => {}
        }
    }
    if sheets.is_empty() {
        return Err(error::xlsx_format("workbook.xml has no sheets"));
    }
    Ok(sheets)
}

fn parse_name_ref(text: &str) -> NameReferent {
    if let Ok(parsed) = parse_a1(text) {
        match parsed.kind {
            RefKind::Range(r) => return NameReferent::Range(r),
            RefKind::Cell(c) => {
                return NameReferent::Range(RangeRef::from_corners(c, c));
            }
        }
    }
    NameReferent::Formula(text.to_string())
}

fn truthy(s: &str) -> bool {
    matches!(s, "1" | "true" | "True" | "TRUE")
}

struct Sst(Vec<(String, Vec<RichTextRun>)>);

fn load_sst(
    package: &OpcPackage,
    rels: &[Relationship],
    warnings: &mut FileWarnings,
) -> Result<Sst, CoreError> {
    let Some(rel) = rels.iter().find(|r| r.rel_type == REL_SST) else {
        return Ok(Sst(Vec::new()));
    };
    let Some(part) = package.part(&rel.target) else {
        warnings.push(
            "xlsx.part",
            "sharedStrings relationship is dangling",
            Some(rel.target.clone()),
        );
        return Ok(Sst(Vec::new()));
    };
    let mut r = XmlReader::new(&part.bytes);
    let mut out = Vec::new();
    let mut in_si = false;
    let mut in_t = false;
    let mut text = String::new();
    let mut runs: Vec<RichTextRun> = Vec::new();
    let mut run_start = 0u32;
    let mut run_font = Font::default();
    let mut in_r = false;
    let mut in_rpr = false;
    while let Some(ev) = r.next()? {
        match ev {
            XmlEvent::Start { name, .. } if name == "si" => {
                in_si = true;
                text.clear();
                runs.clear();
            }
            XmlEvent::End { name } if name == "si" => {
                in_si = false;
                out.push((text.clone(), runs.clone()));
            }
            XmlEvent::Start { name, attrs } if in_si && name == "t" => {
                in_t = true;
                let _ = attrs;
            }
            XmlEvent::End { name } if name == "t" => in_t = false,
            XmlEvent::Start { name, .. } if in_si && name == "r" => {
                in_r = true;
                run_start = text.len() as u32;
                run_font = Font::default();
            }
            XmlEvent::Start { name, .. } if in_r && name == "rPr" => in_rpr = true,
            XmlEvent::End { name } if name == "rPr" => in_rpr = false,
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs } if in_rpr => {
                apply_font_tag(&mut run_font, &name, &attrs);
            }
            XmlEvent::End { name } if in_r && name == "r" => {
                in_r = false;
                let len = (text.len() as u32).saturating_sub(run_start);
                if len > 0 {
                    runs.push(RichTextRun {
                        start: run_start,
                        len,
                        font: run_font.clone(),
                    });
                }
            }
            XmlEvent::Text(t) if in_t => text.push_str(&t),
            _ => {}
        }
    }
    Ok(Sst(out))
}

fn apply_font_tag(font: &mut Font, name: &str, attrs: &[(String, String)]) {
    match name {
        "b" => font.bold = attr(attrs, "val").is_none_or(truthy),
        "i" => font.italic = attr(attrs, "val").is_none_or(truthy),
        "strike" => font.strike = attr(attrs, "val").is_none_or(truthy),
        "sz" => {
            if let Some(v) = attr(attrs, "val").and_then(|s| s.parse().ok()) {
                font.size_pt = v;
            }
        }
        "name" => {
            if let Some(v) = attr(attrs, "val") {
                font.name = v.to_string();
            }
        }
        "u" => {
            font.underline = match attr(attrs, "val").unwrap_or("single") {
                "double" => Underline::Double,
                "singleAccounting" => Underline::SingleAccounting,
                "doubleAccounting" => Underline::DoubleAccounting,
                "none" => Underline::None,
                _ => Underline::Single,
            };
        }
        "color" => font.color = parse_color(attrs, None),
        _ => {}
    }
}

struct StyleTable {
    cell_xfs: Vec<Style>,
}

fn load_styles(
    package: &OpcPackage,
    rels: &[Relationship],
    theme: &Theme,
    warnings: &mut FileWarnings,
) -> Result<StyleTable, CoreError> {
    let Some(rel) = rels.iter().find(|r| r.rel_type == REL_STYLES) else {
        return Ok(StyleTable {
            cell_xfs: vec![Style::default()],
        });
    };
    let Some(part) = package.part(&rel.target) else {
        warnings.push(
            "xlsx.part",
            "styles relationship is dangling",
            Some(rel.target.clone()),
        );
        return Ok(StyleTable {
            cell_xfs: vec![Style::default()],
        });
    };
    let mut r = XmlReader::new(&part.bytes);
    let mut numfmts: HashMap<u32, String> = HashMap::new();
    let mut fonts: Vec<Font> = Vec::new();
    let mut fills: Vec<Fill> = Vec::new();
    let mut borders: Vec<Border> = Vec::new();
    let mut cell_xfs: Vec<Style> = Vec::new();
    let mut in_numfmts = false;
    let mut in_fonts = false;
    let mut in_fills = false;
    let mut in_borders = false;
    let mut in_xfs = false;
    let mut in_font = false;
    let mut cur_font = Font::default();
    let mut in_fill = false;
    let mut cur_fill = Fill::None;
    let mut in_gradient = false;
    let mut grad = GradientFill::default();
    let mut in_border = false;
    let mut cur_border = Border::default();
    let mut border_side = "";
    while let Some(ev) = r.next()? {
        match ev {
            XmlEvent::Start { name, .. } if name == "numFmts" => in_numfmts = true,
            XmlEvent::End { name } if name == "numFmts" => in_numfmts = false,
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if in_numfmts && name == "numFmt" =>
            {
                if let (Some(id), Some(code)) = (
                    attr(&attrs, "numFmtId").and_then(|s| s.parse().ok()),
                    attr(&attrs, "formatCode"),
                ) {
                    numfmts.insert(id, code.to_string());
                }
            }
            XmlEvent::Start { name, .. } if name == "fonts" => in_fonts = true,
            XmlEvent::End { name } if name == "fonts" => in_fonts = false,
            XmlEvent::Start { name, .. } if in_fonts && name == "font" => {
                in_font = true;
                cur_font = Font::default();
            }
            XmlEvent::End { name } if in_font && name == "font" => {
                in_font = false;
                fonts.push(cur_font.clone());
            }
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if in_font && name != "font" =>
            {
                apply_font_tag(&mut cur_font, &name, &attrs);
            }
            XmlEvent::Start { name, .. } if name == "fills" => in_fills = true,
            XmlEvent::End { name } if name == "fills" => in_fills = false,
            XmlEvent::Start { name, .. } if in_fills && name == "fill" => {
                in_fill = true;
                cur_fill = Fill::None;
            }
            XmlEvent::End { name } if in_fill && name == "fill" => {
                in_fill = false;
                fills.push(cur_fill.clone());
            }
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if in_fill && name == "patternFill" =>
            {
                let pat = attr(&attrs, "patternType").unwrap_or("none");
                if pat == "solid" {
                    cur_fill = Fill::Solid { fg: Color::Auto };
                } else if pat != "none" {
                    cur_fill = Fill::Pattern {
                        pattern: parse_pattern(pat),
                        fg: Color::Auto,
                        bg: Color::Auto,
                    };
                }
            }
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if in_fill && (name == "fgColor" || name == "bgColor") =>
            {
                let c = parse_color(&attrs, Some(theme));
                match &mut cur_fill {
                    Fill::Solid { fg } if name == "fgColor" => *fg = c,
                    Fill::Pattern { fg, .. } if name == "fgColor" => *fg = c,
                    Fill::Pattern { bg, .. } if name == "bgColor" => *bg = c,
                    _ => {}
                }
            }
            XmlEvent::Start { name, attrs } if in_fill && name == "gradientFill" => {
                in_gradient = true;
                grad = GradientFill {
                    kind: if attr(&attrs, "type").is_some_and(|t| t == "path") {
                        GradientKind::Path
                    } else {
                        GradientKind::Linear
                    },
                    degree: attr(&attrs, "degree")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0),
                    ..GradientFill::default()
                };
            }
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if in_gradient && name == "stop" =>
            {
                let pos = attr(&attrs, "position")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.0);
                grad.stops.push(GradientStop {
                    position: pos,
                    color: Color::Auto,
                });
            }
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if in_gradient && name == "color" =>
            {
                if let Some(last) = grad.stops.last_mut() {
                    last.color = parse_color(&attrs, Some(theme));
                }
            }
            XmlEvent::End { name } if name == "gradientFill" => {
                in_gradient = false;
                cur_fill = Fill::Gradient(grad.clone());
            }
            XmlEvent::Start { name, .. } if name == "borders" => in_borders = true,
            XmlEvent::End { name } if name == "borders" => in_borders = false,
            XmlEvent::Start { name, .. } if in_borders && name == "border" => {
                in_border = true;
                cur_border = Border::default();
            }
            XmlEvent::End { name } if in_border && name == "border" => {
                in_border = false;
                borders.push(cur_border);
            }
            XmlEvent::Start { name, attrs } | XmlEvent::Empty { name, attrs }
                if in_border && matches!(name.as_str(), "left" | "right" | "top" | "bottom") =>
            {
                border_side = match name.as_str() {
                    "left" => "left",
                    "right" => "right",
                    "top" => "top",
                    _ => "bottom",
                };
                let style = parse_border_style(attr(&attrs, "style").unwrap_or("none"));
                let side = BorderSide {
                    style,
                    color: Color::Auto,
                };
                match border_side {
                    "left" => cur_border.left = side,
                    "right" => cur_border.right = side,
                    "top" => cur_border.top = side,
                    _ => cur_border.bottom = side,
                }
            }
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if in_border && name == "color" =>
            {
                let c = parse_color(&attrs, Some(theme));
                match border_side {
                    "left" => cur_border.left.color = c,
                    "right" => cur_border.right.color = c,
                    "top" => cur_border.top.color = c,
                    _ => cur_border.bottom.color = c,
                }
            }
            XmlEvent::Start { name, .. } if name == "cellXfs" => in_xfs = true,
            XmlEvent::End { name } if name == "cellXfs" => in_xfs = false,
            XmlEvent::Start { name, attrs } | XmlEvent::Empty { name, attrs }
                if in_xfs && name == "xf" =>
            {
                cell_xfs.push(xf_to_style(&attrs, &fonts, &fills, &borders, &numfmts));
            }
            XmlEvent::Start { name, attrs } if in_xfs && name == "alignment" => {
                if let Some(last) = cell_xfs.last_mut() {
                    last.alignment = parse_alignment(&attrs);
                }
            }
            XmlEvent::Empty { name, attrs } if in_xfs && name == "alignment" => {
                if let Some(last) = cell_xfs.last_mut() {
                    last.alignment = parse_alignment(&attrs);
                }
            }
            XmlEvent::Start { name, attrs } | XmlEvent::Empty { name, attrs }
                if in_xfs && name == "protection" =>
            {
                if let Some(last) = cell_xfs.last_mut() {
                    last.protection = Protection {
                        locked: attr(&attrs, "locked").map(truthy).unwrap_or(true),
                        hidden: attr(&attrs, "hidden").is_some_and(truthy),
                    };
                }
            }
            _ => {}
        }
    }
    if cell_xfs.is_empty() {
        cell_xfs.push(Style::default());
    }
    Ok(StyleTable { cell_xfs })
}

fn xf_to_style(
    attrs: &[(String, String)],
    fonts: &[Font],
    fills: &[Fill],
    borders: &[Border],
    numfmts: &HashMap<u32, String>,
) -> Style {
    let font_id = attr(attrs, "fontId")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let fill_id = attr(attrs, "fillId")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let border_id = attr(attrs, "borderId")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let num_id = attr(attrs, "numFmtId")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    let mut style = Style::default();
    if let Some(f) = fonts.get(font_id) {
        style.font = f.clone();
    }
    if let Some(f) = fills.get(fill_id) {
        style.fill = f.clone();
    }
    if let Some(b) = borders.get(border_id) {
        style.border = *b;
    }
    style.num_fmt = NumFmtId::new(num_id);
    let _ = numfmts;
    style
}

fn parse_alignment(attrs: &[(String, String)]) -> Alignment {
    Alignment {
        horizontal: match attr(attrs, "horizontal").unwrap_or("general") {
            "left" => HorizontalAlign::Left,
            "center" => HorizontalAlign::Center,
            "right" => HorizontalAlign::Right,
            "fill" => HorizontalAlign::Fill,
            "justify" => HorizontalAlign::Justify,
            "centerContinuous" => HorizontalAlign::CenterContinuous,
            "distributed" => HorizontalAlign::Distributed,
            _ => HorizontalAlign::General,
        },
        vertical: match attr(attrs, "vertical").unwrap_or("bottom") {
            "top" => VerticalAlign::Top,
            "center" => VerticalAlign::Center,
            "justify" => VerticalAlign::Justify,
            "distributed" => VerticalAlign::Distributed,
            _ => VerticalAlign::Bottom,
        },
        wrap: attr(attrs, "wrapText").is_some_and(truthy),
        shrink: attr(attrs, "shrinkToFit").is_some_and(truthy),
        indent: attr(attrs, "indent")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        rotation: attr(attrs, "textRotation")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
    }
}

fn parse_pattern(s: &str) -> PatternType {
    match s {
        "solid" => PatternType::Solid,
        "mediumGray" => PatternType::MediumGray,
        "darkGray" => PatternType::DarkGray,
        "lightGray" => PatternType::LightGray,
        "gray125" => PatternType::Gray125,
        "gray0625" => PatternType::Gray0625,
        _ => PatternType::None,
    }
}

fn parse_border_style(s: &str) -> BorderStyle {
    match s {
        "thin" => BorderStyle::Thin,
        "medium" => BorderStyle::Medium,
        "dashed" => BorderStyle::Dashed,
        "dotted" => BorderStyle::Dotted,
        "thick" => BorderStyle::Thick,
        "double" => BorderStyle::Double,
        "hair" => BorderStyle::Hair,
        "mediumDashed" => BorderStyle::MediumDashed,
        "dashDot" => BorderStyle::DashDot,
        "mediumDashDot" => BorderStyle::MediumDashDot,
        "dashDotDot" => BorderStyle::DashDotDot,
        "mediumDashDotDot" => BorderStyle::MediumDashDotDot,
        "slantDashDot" => BorderStyle::SlantDashDot,
        _ => BorderStyle::None,
    }
}

#[derive(Clone, Debug, Default)]
struct Theme {
    scheme: Vec<Color>,
}

fn load_theme(package: &OpcPackage, rels: &[Relationship], warnings: &mut FileWarnings) -> Theme {
    let Some(rel) = rels.iter().find(|r| r.rel_type == REL_THEME) else {
        return Theme::default();
    };
    let Some(part) = package.part(&rel.target) else {
        warnings.push(
            "xlsx.part",
            "theme relationship is dangling",
            Some(rel.target.clone()),
        );
        return Theme::default();
    };
    let mut r = XmlReader::new(&part.bytes);
    let mut scheme = Vec::new();
    let mut in_scheme = false;
    while let Ok(Some(ev)) = r.next() {
        match ev {
            XmlEvent::Start { name, .. } if name == "clrScheme" => in_scheme = true,
            XmlEvent::End { name } if name == "clrScheme" => in_scheme = false,
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if in_scheme && (name == "srgbClr" || name == "sysClr") =>
            {
                if let Some(val) = attr(&attrs, "val").or_else(|| attr(&attrs, "lastClr")) {
                    scheme.push(rgb_from_hex(val));
                }
            }
            _ => {}
        }
    }
    Theme { scheme }
}

fn parse_color(attrs: &[(String, String)], theme: Option<&Theme>) -> Color {
    if let Some(rgb) = attr(attrs, "rgb") {
        return rgb_from_hex(rgb);
    }
    if let Some(idx) = attr(attrs, "theme").and_then(|s| s.parse::<usize>().ok()) {
        let tint = attr(attrs, "tint")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        if let Some(t) = theme
            && let Some(c) = t.scheme.get(idx)
        {
            let _ = tint;
            return *c;
        }
        return Color::Theme {
            theme: idx as u8,
            tint,
        };
    }
    if let Some(i) = attr(attrs, "indexed").and_then(|s| s.parse().ok()) {
        return Color::Indexed { index: i };
    }
    Color::Auto
}

fn rgb_from_hex(s: &str) -> Color {
    let t = s.trim().trim_start_matches('#');
    let padded = if t.len() == 6 {
        format!("FF{t}")
    } else {
        t.to_string()
    };
    let argb = u32::from_str_radix(&padded, 16).unwrap_or(0xFF00_0000);
    Color::Rgb { argb }
}

#[allow(clippy::too_many_arguments)]
fn load_sheet(
    wb: &mut Workbook,
    id: omacell_core::addr::SheetId,
    package: &OpcPackage,
    part_name: &str,
    sheet_rels: &[Relationship],
    sst: &Sst,
    styles: &StyleTable,
    warnings: &mut FileWarnings,
) -> Result<WorksheetExtras, CoreError> {
    let Some(part) = package.part(part_name) else {
        warnings.push(
            "xlsx.part",
            "worksheet part missing",
            Some(part_name.into()),
        );
        return Ok(WorksheetExtras::default());
    };
    let mut extra = WorksheetExtras::default();
    let mut r = XmlReader::new(&part.bytes);
    let mut in_sheet_data = false;
    let mut in_c = false;
    let mut in_v = false;
    let mut in_f = false;
    let mut in_is = false;
    let mut in_t = false;
    let mut cell_ref = String::new();
    let mut cell_type = String::new();
    let mut cell_style: Option<usize> = None;
    let mut v_text = String::new();
    let mut f_text = String::new();
    let mut f_t = String::new();
    let mut f_si: Option<u32> = None;
    let mut f_ref = String::new();
    let mut is_text = String::new();
    let mut shared: HashMap<u32, (u32, u16, String)> = HashMap::new();
    let mut merges: Vec<RangeRef> = Vec::new();
    let mut in_hyperlinks = false;

    while let Some(ev) = r.next()? {
        match ev {
            XmlEvent::Start { name, .. } if name == "sheetData" => in_sheet_data = true,
            XmlEvent::End { name } if name == "sheetData" => in_sheet_data = false,
            XmlEvent::Start { name, attrs } if in_sheet_data && name == "row" => {
                if let Some(ridx) = attr(&attrs, "r").and_then(|s| s.parse::<u32>().ok()) {
                    let row = ridx.saturating_sub(1);
                    if attr(&attrs, "hidden").is_some_and(truthy) {
                        let _ = wb.set_row_hidden(id, row, true);
                    }
                    if let Some(ht) = attr(&attrs, "ht").and_then(|s| s.parse::<f64>().ok()) {
                        let px = (ht * 96.0 / 72.0).round().max(1.0) as u32;
                        let _ = wb.sheet_mut(id)?.geometry.rows.set_size(row, px);
                    }
                }
            }
            XmlEvent::Start { name, attrs } if in_sheet_data && name == "c" => {
                in_c = true;
                cell_ref = attr(&attrs, "r").unwrap_or("").to_string();
                cell_type = attr(&attrs, "t").unwrap_or("n").to_string();
                cell_style = attr(&attrs, "s").and_then(|s| s.parse().ok());
                v_text.clear();
                f_text.clear();
                f_t.clear();
                f_si = None;
                f_ref.clear();
                is_text.clear();
            }
            XmlEvent::End { name } if in_c && name == "c" => {
                in_c = false;
                if let Err(e) = commit_cell(
                    wb,
                    id,
                    &cell_ref,
                    &cell_type,
                    cell_style,
                    &v_text,
                    &f_text,
                    &f_t,
                    f_si,
                    &f_ref,
                    &is_text,
                    sst,
                    styles,
                    &mut shared,
                    warnings,
                    part_name,
                ) {
                    warnings.push("xlsx.cell", e.message, Some(part_name.into()));
                }
            }
            XmlEvent::Start { name, attrs } if in_c && name == "f" => {
                in_f = true;
                f_t = attr(&attrs, "t").unwrap_or("").to_string();
                f_si = attr(&attrs, "si").and_then(|s| s.parse().ok());
                f_ref = attr(&attrs, "ref").unwrap_or("").to_string();
            }
            XmlEvent::Empty { name, attrs } if in_c && name == "f" => {
                f_t = attr(&attrs, "t").unwrap_or("").to_string();
                f_si = attr(&attrs, "si").and_then(|s| s.parse().ok());
                f_ref = attr(&attrs, "ref").unwrap_or("").to_string();
            }
            XmlEvent::End { name } if name == "f" => in_f = false,
            XmlEvent::Start { name, .. } if in_c && name == "v" => in_v = true,
            XmlEvent::End { name } if name == "v" => in_v = false,
            XmlEvent::Start { name, .. } if in_c && name == "is" => in_is = true,
            XmlEvent::End { name } if name == "is" => in_is = false,
            XmlEvent::Start { name, .. } if in_is && name == "t" => in_t = true,
            XmlEvent::End { name } if name == "t" => in_t = false,
            XmlEvent::Text(t) if in_f => f_text.push_str(&t),
            XmlEvent::Text(t) if in_v => v_text.push_str(&t),
            XmlEvent::Text(t) if in_t => is_text.push_str(&t),
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if name == "mergeCell" =>
            {
                if let Some(rf) = attr(&attrs, "ref")
                    && let Ok(parsed) = parse_a1(rf)
                {
                    match parsed.kind {
                        RefKind::Range(rg) => merges.push(rg),
                        RefKind::Cell(c) => merges.push(RangeRef::from_corners(c, c)),
                    }
                }
            }
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs } if name == "col" => {
                let min = attr(&attrs, "min")
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(1);
                let max = attr(&attrs, "max")
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(min);
                let hidden = attr(&attrs, "hidden").is_some_and(truthy);
                let width = attr(&attrs, "width").and_then(|s| s.parse::<f64>().ok());
                let sheet = wb.sheet_mut(id)?;
                for col in min..=max {
                    let idx = (col.saturating_sub(1)) as u16;
                    if hidden {
                        let _ = sheet.geometry.cols.set_hidden(u32::from(idx), true);
                    }
                    if let Some(w) = width {
                        let px = (w * f64::from(omacell_core::geometry::DEFAULT_COL_PX) / 8.43)
                            .round()
                            .max(1.0) as u32;
                        let _ = sheet.geometry.cols.set_size(u32::from(idx), px);
                    }
                }
            }
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs } if name == "pane" => {
                let y = attr(&attrs, "ySplit")
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let x = attr(&attrs, "xSplit")
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let state = attr(&attrs, "state").unwrap_or("");
                let view = &mut wb.sheet_mut(id)?.view;
                if state == "frozen" || state == "frozenSplit" {
                    view.freeze = FreezePanes {
                        rows: y.round().max(0.0) as u32,
                        cols: x.round().max(0.0) as u16,
                    };
                } else if x > 0.0 || y > 0.0 {
                    view.split = Some(SplitView {
                        x_px: x.round().max(0.0) as u32,
                        y_px: y.round().max(0.0) as u32,
                    });
                }
            }
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if name == "sheetView" =>
            {
                if let Some(z) = attr(&attrs, "zoomScale").and_then(|s| s.parse::<f64>().ok()) {
                    wb.sheet_mut(id)?.view.zoom = z / 100.0;
                }
                if attr(&attrs, "showGridLines").is_some_and(|s| !truthy(s)) {
                    wb.sheet_mut(id)?.view.gridlines = false;
                }
                if attr(&attrs, "showFormulas").is_some_and(truthy) {
                    wb.sheet_mut(id)?.view.show_formulas = true;
                }
            }
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if name == "selection" =>
            {
                if let Some(sq) = attr(&attrs, "sqref").and_then(|s| s.split_whitespace().next())
                    && let Ok(parsed) = parse_a1(sq)
                {
                    wb.sheet_mut(id)?.view.selection = match parsed.kind {
                        RefKind::Range(rg) => rg,
                        RefKind::Cell(c) => RangeRef::from_corners(c, c),
                    };
                }
            }
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if name == "autoFilter" =>
            {
                extra.autofilter = attr(&attrs, "ref").map(ToOwned::to_owned);
            }
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if name == "sheetProtection" =>
            {
                wb.sheet_mut(id)?.protection = ProtectionState {
                    enabled: true,
                    password: attr(&attrs, "hashValue")
                        .or_else(|| attr(&attrs, "password"))
                        .map(|s| s.as_bytes().to_vec()),
                };
            }
            XmlEvent::Start { name, .. } if name == "hyperlinks" => in_hyperlinks = true,
            XmlEvent::End { name } if name == "hyperlinks" => in_hyperlinks = false,
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if in_hyperlinks && name == "hyperlink" =>
            {
                if let Some(rf) = attr(&attrs, "ref")
                    && let Ok(cell) = parse_a1_cell(rf.split(':').next().unwrap_or(rf))
                {
                    let rid = attr(&attrs, "id");
                    let loc = attr(&attrs, "location");
                    let tooltip = attr(&attrs, "tooltip").map(ToOwned::to_owned);
                    let display = attr(&attrs, "display").map(ToOwned::to_owned);
                    let target = if let Some(id) = rid {
                        sheet_rels
                            .iter()
                            .find(|rel| rel.id == id && rel.rel_type == REL_HYPER)
                            .map(|rel| rel.target.clone())
                            .unwrap_or_default()
                    } else {
                        loc.unwrap_or("").to_string()
                    };
                    let _ = wb.set_hyperlink(
                        id,
                        cell.row,
                        cell.col,
                        Some(Hyperlink {
                            target,
                            tooltip,
                            display,
                        }),
                    );
                }
            }
            XmlEvent::Start { name, attrs } if name == "conditionalFormatting" => {
                extra
                    .conditional_formatting_xml
                    .push(format!("ref={}", attr(&attrs, "ref").unwrap_or("")));
            }
            XmlEvent::Start { name, .. } if name == "dataValidations" => {
                extra.data_validations_xml.push("dataValidations".into());
            }
            _ => {}
        }
    }
    wb.sheet_mut(id)?.merges = merges;
    Ok(extra)
}

#[allow(clippy::too_many_arguments)]
fn commit_cell(
    wb: &mut Workbook,
    sheet: omacell_core::addr::SheetId,
    r: &str,
    t: &str,
    style_idx: Option<usize>,
    v: &str,
    f: &str,
    f_t: &str,
    f_si: Option<u32>,
    f_ref: &str,
    inline: &str,
    sst: &Sst,
    styles: &StyleTable,
    shared: &mut HashMap<u32, (u32, u16, String)>,
    warnings: &mut FileWarnings,
    part: &str,
) -> Result<(), CoreError> {
    if r.is_empty() {
        return Ok(());
    }
    let cell = parse_a1_cell(r)?;
    let mut formula_src: Option<String> = None;
    if f_t == "shared" {
        if let Some(si) = f_si {
            if !f.is_empty() {
                let src = with_eq(f);
                shared.insert(si, (cell.row, cell.col, src.clone()));
                formula_src = Some(src);
                let _ = f_ref;
            } else if let Some(&(mr, mc, ref master)) = shared.get(&si) {
                formula_src = Some(shift_formula(
                    master,
                    cell.row as i32 - mr as i32,
                    cell.col as i32 - i32::from(mc),
                    warnings,
                    part,
                ));
            }
        }
    } else if !f.is_empty() {
        formula_src = Some(with_eq(f));
    }

    if let Some(src) = formula_src.as_ref() {
        match parse(src) {
            Ok(_) => {
                wb.set_formula_text(sheet, cell.row, cell.col, src)?;
            }
            Err(e) => {
                warnings.push(
                    "xlsx.formula",
                    format!("unparsable formula at {r}: {}", e.error.message),
                    Some(part.into()),
                );
                wb.set_text(sheet, cell.row, cell.col, src)?;
                apply_style(wb, sheet, cell, style_idx, styles)?;
                return Ok(());
            }
        }
        if !v.is_empty() && t != "s" && t != "inlineStr" {
            if let Ok(n) = v.parse::<f64>() {
                let mut slot = wb
                    .get(sheet, cell.row, cell.col)?
                    .copied()
                    .unwrap_or_else(CellSlot::empty);
                slot.value = Value::Number(n);
                wb.set_slot(sheet, cell.row, cell.col, slot)?;
            } else if t == "b" {
                let mut slot = wb
                    .get(sheet, cell.row, cell.col)?
                    .copied()
                    .unwrap_or_else(CellSlot::empty);
                slot.value = Value::Bool(truthy(v) || v == "1");
                wb.set_slot(sheet, cell.row, cell.col, slot)?;
            }
        }
        apply_style(wb, sheet, cell, style_idx, styles)?;
        return Ok(());
    }

    match t {
        "s" => {
            let idx: usize = v.trim().parse().unwrap_or(0);
            if let Some((text, runs)) = sst.0.get(idx) {
                if runs.is_empty() {
                    wb.set_text(sheet, cell.row, cell.col, text)?;
                } else {
                    let sid = wb.intern_rich_text(text, runs.clone());
                    let slot = CellSlot {
                        value: Value::Text(sid),
                        formula: None,
                        style: omacell_core::style::StyleId::DEFAULT,
                        flags: CellFlags::DEFAULT,
                    };
                    wb.set_slot(sheet, cell.row, cell.col, slot)?;
                    wb.release_text(sid);
                }
            }
        }
        "inlineStr" | "str" => {
            let text = if t == "str" { v } else { inline };
            if !text.is_empty() {
                wb.set_text(sheet, cell.row, cell.col, text)?;
            }
        }
        "b" => {
            let slot = CellSlot {
                value: Value::Bool(truthy(v) || v == "1"),
                formula: None,
                style: omacell_core::style::StyleId::DEFAULT,
                flags: CellFlags::DEFAULT,
            };
            wb.set_slot(sheet, cell.row, cell.col, slot)?;
        }
        "e" => {
            let kind = ErrorKind::from_display(v.trim()).unwrap_or(ErrorKind::Value);
            let slot = CellSlot {
                value: Value::Error(kind),
                formula: None,
                style: omacell_core::style::StyleId::DEFAULT,
                flags: CellFlags::DEFAULT,
            };
            wb.set_slot(sheet, cell.row, cell.col, slot)?;
        }
        "d" => {
            if let Some(serial) = parse_iso_date(v, wb.settings().date_system) {
                wb.set_number(sheet, cell.row, cell.col, serial)?;
            } else {
                wb.set_text(sheet, cell.row, cell.col, v)?;
            }
        }
        _ => {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                if let Ok(n) = trimmed.parse::<f64>() {
                    wb.set_number(sheet, cell.row, cell.col, n)?;
                } else {
                    wb.set_text(sheet, cell.row, cell.col, trimmed)?;
                }
            }
        }
    }
    apply_style(wb, sheet, cell, style_idx, styles)?;
    Ok(())
}

fn apply_style(
    wb: &mut Workbook,
    sheet: omacell_core::addr::SheetId,
    cell: CellRef,
    style_idx: Option<usize>,
    styles: &StyleTable,
) -> Result<(), CoreError> {
    let Some(i) = style_idx else {
        return Ok(());
    };
    if i == 0 {
        return Ok(());
    }
    let Some(style) = styles.cell_xfs.get(i).cloned() else {
        return Ok(());
    };
    if style == Style::default() {
        return Ok(());
    }
    wb.set_cell_style(sheet, cell.row, cell.col, style)?;
    Ok(())
}

fn with_eq(f: &str) -> String {
    let t = f.trim();
    if t.starts_with('=') {
        t.to_string()
    } else {
        format!("={t}")
    }
}

fn shift_formula(
    master: &str,
    drow: i32,
    dcol: i32,
    warnings: &mut FileWarnings,
    part: &str,
) -> String {
    match parse(master) {
        Ok(f) => {
            let ast = copy_delta(&f.ast, drow, dcol);
            let shifted = omacell_core::formula::Formula {
                ast,
                style: f.style,
                base_row: f.base_row,
                base_col: f.base_col,
            };
            print(&shifted)
        }
        Err(e) => {
            warnings.push(
                "xlsx.formula",
                format!("shared formula master failed to parse: {}", e.error.message),
                Some(part.into()),
            );
            master.to_string()
        }
    }
}

fn parse_iso_date(s: &str, system: DateSystem) -> Option<f64> {
    let (d, t) = s.split_once('T').unwrap_or((s, ""));
    let mut p = d.split('-');
    let y: i32 = p.next()?.parse().ok()?;
    let m: u8 = p.next()?.parse().ok()?;
    let day: u8 = p.next()?.parse().ok()?;
    let date = omacell_core::dates::CivilDate {
        year: y,
        month: m,
        day,
        lotus_leap: false,
    };
    let mut serial = omacell_core::dates::date_to_serial(date, system)? as f64;
    if !t.is_empty() {
        let mut tp = t.trim_end_matches('Z').split(':');
        let h: f64 = tp.next()?.parse().ok()?;
        let min: f64 = tp.next()?.parse().ok()?;
        let sec: f64 = tp.next().unwrap_or("0").parse().ok()?;
        serial += (h * 3600.0 + min * 60.0 + sec) / 86_400.0;
    }
    Some(serial)
}

fn load_tables(
    wb: &mut Workbook,
    sheet: omacell_core::addr::SheetId,
    package: &OpcPackage,
    rels: &[Relationship],
    warnings: &mut FileWarnings,
) -> Result<(), CoreError> {
    for rel in rels.iter().filter(|r| r.rel_type == REL_TABLE) {
        let Some(part) = package.part(&rel.target) else {
            warnings.push("xlsx.part", "table part missing", Some(rel.target.clone()));
            continue;
        };
        let mut r = XmlReader::new(&part.bytes);
        let mut name = String::new();
        let mut rf = String::new();
        let mut header = true;
        let mut totals = false;
        let mut cols: Vec<String> = Vec::new();
        while let Some(ev) = r.next()? {
            match ev {
                XmlEvent::Start { name: n, attrs } | XmlEvent::Empty { name: n, attrs }
                    if n == "table" =>
                {
                    name = attr(&attrs, "name")
                        .or_else(|| attr(&attrs, "displayName"))
                        .unwrap_or("Table")
                        .to_string();
                    rf = attr(&attrs, "ref").unwrap_or("").to_string();
                    header = attr(&attrs, "headerRowCount")
                        .map(|s| s != "0")
                        .unwrap_or(true);
                    totals = attr(&attrs, "totalsRowCount").is_some_and(|s| s != "0");
                }
                XmlEvent::Empty { name: n, attrs } | XmlEvent::Start { name: n, attrs }
                    if n == "tableColumn" =>
                {
                    cols.push(attr(&attrs, "name").unwrap_or("Column").to_string());
                }
                _ => {}
            }
        }
        if let Ok(parsed) = parse_a1(&rf) {
            let (sr, sc, er, ec) = match parsed.kind {
                RefKind::Range(rg) => (rg.start.row, rg.start.col, rg.end.row, rg.end.col),
                RefKind::Cell(c) => (c.row, c.col, c.row, c.col),
            };
            let mut table = Table::new(
                omacell_core::tables::TableId::new(0),
                name,
                sheet,
                sr,
                sc,
                er,
                ec,
            );
            table.has_header = header;
            table.has_totals = totals;
            if !cols.is_empty() {
                table.columns = cols.into_iter().map(|name| TableColumn { name }).collect();
            }
            if let Err(e) = wb.add_table(table) {
                warnings.push("xlsx.table", e.message, Some(rel.target.clone()));
            }
        }
    }
    Ok(())
}

fn load_comments(
    wb: &mut Workbook,
    sheet: omacell_core::addr::SheetId,
    package: &OpcPackage,
    rels: &[Relationship],
    warnings: &mut FileWarnings,
) -> Result<(), CoreError> {
    for rel in rels.iter().filter(|r| r.rel_type == REL_COMMENTS) {
        let Some(part) = package.part(&rel.target) else {
            warnings.push(
                "xlsx.part",
                "comments part missing",
                Some(rel.target.clone()),
            );
            continue;
        };
        let mut r = XmlReader::new(&part.bytes);
        let mut authors: Vec<String> = Vec::new();
        let mut in_authors = false;
        let mut in_author = false;
        let mut author_text = String::new();
        let mut in_comment = false;
        let mut comment_ref = String::new();
        let mut comment_author = 0usize;
        let mut in_t = false;
        let mut body = String::new();
        while let Some(ev) = r.next()? {
            match ev {
                XmlEvent::Start { name, .. } if name == "authors" => in_authors = true,
                XmlEvent::End { name } if name == "authors" => in_authors = false,
                XmlEvent::Start { name, .. } if in_authors && name == "author" => {
                    in_author = true;
                    author_text.clear();
                }
                XmlEvent::End { name } if name == "author" => {
                    in_author = false;
                    authors.push(author_text.clone());
                }
                XmlEvent::Text(t) if in_author => author_text.push_str(&t),
                XmlEvent::Start { name, attrs } if name == "comment" => {
                    in_comment = true;
                    comment_ref = attr(&attrs, "ref").unwrap_or("").to_string();
                    comment_author = attr(&attrs, "authorId")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    body.clear();
                }
                XmlEvent::End { name } if name == "comment" => {
                    in_comment = false;
                    if let Ok(cell) = parse_a1_cell(&comment_ref) {
                        let author = authors.get(comment_author).cloned();
                        let _ = wb.set_note(
                            sheet,
                            cell.row,
                            cell.col,
                            Some(Note {
                                author,
                                text: body.trim().to_string(),
                            }),
                        );
                    }
                }
                XmlEvent::Start { name, .. } if in_comment && name == "t" => in_t = true,
                XmlEvent::End { name } if name == "t" => in_t = false,
                XmlEvent::Text(t) if in_t => body.push_str(&t),
                _ => {}
            }
        }
    }
    Ok(())
}

fn load_omacell_parts(wb: &mut Workbook, package: &OpcPackage) {
    for (name, part) in &package.parts {
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("xl/omacell/") && !lower.ends_with(".rels") {
            wb.custom_parts.insert(name.clone(), part.bytes.clone());
        }
    }
}
