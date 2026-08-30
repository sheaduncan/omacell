//! Load an OPC package into a [`Workbook`].

use std::collections::{BTreeMap, HashMap};

use omacell_core::addr::{CellRef, RangeRef, RefKind, parse_a1, parse_a1_cell};
use omacell_core::condfmt::CfDxf;
use omacell_core::error::{CoreError, ErrorKind};
use omacell_core::formula::{copy_delta, parse, print};
use omacell_core::intern::RichTextRun;
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use omacell_core::names::{DefinedName, NameReferent, NameScope};
use omacell_core::sheet::{
    FreezePanes, Hyperlink, Note, ProtectedRange, ProtectionState, SheetVisibility, SplitView,
};
use omacell_core::storage::{CellFlags, CellSlot};
use omacell_core::style::{
    Alignment, Border, BorderSide, BorderStyle, Color, Fill, Font, GradientFill, GradientKind,
    GradientStop, HorizontalAlign, NumFmtId, PatternType, Protection, Style, Underline,
    VerticalAlign,
};
use omacell_core::tables::{Table, TableColumn};
use omacell_core::value::Value;
use omacell_core::workbook::{CalcMode, DateSystem, Workbook, WorkbookProtectionState};

use super::XlsxDocument;
use super::opc::{OpcPackage, Relationship, open_package};
use super::warnings::FileWarnings;
use super::xml::{XmlEvent, XmlReader, attr, decode_ooxml_text};
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
const REL_THREADED_COMMENTS: &str =
    "http://schemas.microsoft.com/office/2017/10/relationships/threadedComment";
const REL_PERSON: &str = "http://schemas.microsoft.com/office/2017/10/relationships/person";
const REL_HYPER: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
const REL_DRAWING: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing";
const REL_CHART: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart";

/// Unmodeled worksheet fragments for WP-10.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorksheetExtras {
    /// AutoFilter `ref`.
    pub autofilter: Option<String>,
    /// Raw `autoFilter` XML, retained until the modeled filter changes.
    pub autofilter_xml: Vec<u8>,
    /// Raw `pageSetup` / `pageMargins` / `printOptions` / `headerFooter` XML.
    pub print_xml: Vec<Vec<u8>>,
    /// Raw `conditionalFormatting` XML blobs.
    pub conditional_formatting_xml: Vec<Vec<u8>>,
    /// Raw `dataValidations` / `extLst` validation XML.
    pub data_validations_xml: Vec<Vec<u8>>,
    /// Sparkline groups (`x14`).
    pub sparkline_xml: Vec<Vec<u8>>,
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
    let theme = load_theme(&package, &wb_rels, &mut warnings)?;
    let sst = load_sst(&package, &wb_rels, &mut warnings)?;
    let styles = load_styles(&package, &wb_rels, &theme, &mut wb, &mut warnings)?;
    let workbook_meta = parse_workbook_xml(&wb_bytes, &mut wb)?;
    let persons = load_persons(&package, &wb_rels, &mut warnings)?;

    let first_id = wb.active_sheet();
    let mut sheet_ids = Vec::with_capacity(workbook_meta.sheets.len());
    for (i, meta) in workbook_meta.sheets.iter().enumerate() {
        let id = if i == 0 {
            wb.rename_sheet(first_id, &meta.name)?;
            first_id
        } else {
            wb.add_sheet(&meta.name)?
        };
        sheet_ids.push(id);
    }
    if !workbook_meta
        .sheets
        .iter()
        .any(|meta| meta.visibility.is_visible())
    {
        return Err(error::xlsx_format(
            "workbook must contain at least one visible sheet",
        ));
    }
    for (meta, &id) in workbook_meta.sheets.iter().zip(&sheet_ids) {
        wb.set_visibility(id, meta.visibility)?;
    }
    let active_index = workbook_meta
        .active_tab
        .filter(|&idx| {
            workbook_meta
                .sheets
                .get(idx)
                .is_some_and(|meta| meta.visibility.is_visible())
        })
        .or_else(|| {
            workbook_meta
                .sheets
                .iter()
                .position(|meta| meta.visibility.is_visible())
        })
        .unwrap_or(0);
    wb.set_active_sheet(sheet_ids[active_index])?;

    for name in workbook_meta.names {
        if name.local_sheet_index.is_some() && super::data::is_filter_database_name(&name.name) {
            continue;
        }
        let scope = match name.local_sheet_index {
            Some(idx) => match sheet_ids.get(idx).copied() {
                Some(id) => NameScope::Sheet(id),
                None => {
                    warnings.push(
                        "xlsx.name",
                        format!("defined name {} has invalid localSheetId {idx}", name.name),
                        Some(wb_name.clone()),
                    );
                    continue;
                }
            },
            None => NameScope::Workbook,
        };
        if let Err(e) = wb.define_name(DefinedName {
            name: name.name,
            scope,
            referent: name.referent,
            comment: name.comment,
        }) {
            warnings.push("xlsx.name", e.message, Some(wb_name.clone()));
        }
    }

    let mut extras: HashMap<String, WorksheetExtras> = HashMap::new();

    for (meta, &id) in workbook_meta.sheets.iter().zip(&sheet_ids) {
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
        let sheet_rels = package.rels_for(&rel.target)?;
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
        let mut setup = omacell_core::print::PageSetup::default();
        super::print::apply_print_xml(&mut setup, &extra.print_xml);
        let _ = wb.set_page_setup(id, setup);
        let validations = super::data::parse_validations(&extra.data_validations_xml);
        if !validations.is_empty() {
            let _ = wb.set_validations(id, validations);
        }
        let cond_formats =
            super::data::parse_cond_formats(&extra.conditional_formatting_xml, &styles.dxfs);
        if !cond_formats.is_empty() {
            let _ = wb.set_cond_formats(id, cond_formats);
        }
        let sparklines = super::drawing::parse_sparklines(&extra.sparkline_xml, &wb, id);
        for sparkline in sparklines {
            let _ = wb.add_sparkline(sparkline);
        }
        extras.insert(meta.name.clone(), extra);
        load_tables(&mut wb, id, &package, &sheet_rels, &mut warnings)?;
        load_comments(&mut wb, id, &package, &sheet_rels, &persons, &mut warnings)?;
        load_charts(&mut wb, id, &package, &sheet_rels);
    }

    apply_print_defined_names(&mut wb);
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
    visibility: SheetVisibility,
}

struct NameMeta {
    name: String,
    local_sheet_index: Option<usize>,
    referent: NameReferent,
    comment: Option<String>,
}

struct WorkbookMeta {
    sheets: Vec<SheetMeta>,
    names: Vec<NameMeta>,
    active_tab: Option<usize>,
}

fn parse_workbook_xml(bytes: &[u8], wb: &mut Workbook) -> Result<WorkbookMeta, CoreError> {
    let mut r = XmlReader::new(bytes);
    let mut sheets = Vec::new();
    let mut names = Vec::new();
    let mut active_tab = None;
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
                    visibility: if state.eq_ignore_ascii_case("veryHidden") {
                        SheetVisibility::VeryHidden
                    } else if state.eq_ignore_ascii_case("hidden") {
                        SheetVisibility::Hidden
                    } else {
                        SheetVisibility::Visible
                    },
                });
            }
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if name == "workbookView" =>
            {
                active_tab = attr(&attrs, "activeTab").and_then(|s| s.parse().ok());
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
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if name == "workbookProtection" =>
            {
                let password =
                    attr(&attrs, "workbookPassword").map(|value| value.as_bytes().to_vec());
                wb.set_workbook_protection(WorkbookProtectionState {
                    enabled: true,
                    lock_structure: attr(&attrs, "lockStructure").is_some_and(truthy),
                    lock_windows: attr(&attrs, "lockWindows").is_some_and(truthy),
                    password,
                })?;
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
                    names.push(NameMeta {
                        name: nm,
                        local_sheet_index: attr(&name_attrs, "localSheetId")
                            .and_then(|s| s.parse::<usize>().ok()),
                        referent: parse_name_ref(name_text.trim(), wb),
                        comment: attr(&name_attrs, "comment").map(ToOwned::to_owned),
                    });
                }
                name_text.clear();
            }
            _ => {}
        }
    }
    if sheets.is_empty() {
        return Err(error::xlsx_format("workbook.xml has no sheets"));
    }
    Ok(WorkbookMeta {
        sheets,
        names,
        active_tab,
    })
}

fn parse_name_ref(text: &str, wb: &mut Workbook) -> NameReferent {
    if let Ok(parsed) = parse_a1(text) {
        match parsed.kind {
            RefKind::Range(r) => return NameReferent::Range(r),
            RefKind::Cell(c) => {
                return NameReferent::Range(RangeRef::from_corners(c, c));
            }
        }
    }
    if let Some(quoted) = text.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        let value = quoted.replace("\"\"", "\"");
        return NameReferent::Constant(Value::Text(wb.intern_text(&value)));
    }
    if text.eq_ignore_ascii_case("TRUE") {
        return NameReferent::Constant(Value::Bool(true));
    }
    if text.eq_ignore_ascii_case("FALSE") {
        return NameReferent::Constant(Value::Bool(false));
    }
    if let Some(error) = ErrorKind::from_display(text) {
        return NameReferent::Constant(Value::Error(error));
    }
    if let Some(number) = text.parse::<f64>().ok().filter(|number| number.is_finite()) {
        return NameReferent::Constant(Value::Number(number));
    }
    NameReferent::Formula(text.to_string())
}

fn truthy(s: &str) -> bool {
    matches!(s, "1" | "true" | "True" | "TRUE")
}

fn apply_print_defined_names(wb: &mut Workbook) {
    let collected: Vec<_> = wb
        .names()
        .iter()
        .map(|n| {
            let text = match &n.referent {
                NameReferent::Range(r) => r.to_a1(),
                NameReferent::Formula(f) => f.clone(),
                NameReferent::Constant(_) => String::new(),
            };
            (n.name.clone(), n.scope, text)
        })
        .collect();
    for (name, scope, text) in collected {
        if text.is_empty() {
            continue;
        }
        let NameScope::Sheet(sheet_id) = scope else {
            continue;
        };
        let Some(sheet) = wb.sheet(sheet_id) else {
            continue;
        };
        let mut setup = sheet.page_setup.clone();
        super::print::apply_print_name(&mut setup, &name, &text);
        let _ = wb.set_page_setup(sheet_id, setup);
    }
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
    let mut in_rph = false;
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
            XmlEvent::Start { name, .. } if in_si && name == "rPh" => in_rph = true,
            XmlEvent::End { name } if name == "rPh" => in_rph = false,
            XmlEvent::Start { name, attrs } if in_si && !in_rph && name == "t" => {
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
            XmlEvent::Text(t) if in_t => text.push_str(&decode_ooxml_text(&t)),
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
        "name" | "rFont" => {
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
    dxfs: Vec<CfDxf>,
}

fn load_styles(
    package: &OpcPackage,
    rels: &[Relationship],
    theme: &Theme,
    wb: &mut Workbook,
    warnings: &mut FileWarnings,
) -> Result<StyleTable, CoreError> {
    let Some(rel) = rels.iter().find(|r| r.rel_type == REL_STYLES) else {
        return Ok(StyleTable {
            cell_xfs: vec![Style::default()],
            dxfs: Vec::new(),
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
            dxfs: Vec::new(),
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
    let mut in_dxfs = false;
    let mut in_dxf = false;
    let mut cur_dxf = CfDxf::default();
    let mut dxfs: Vec<CfDxf> = Vec::new();
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
                    left: attr(&attrs, "left")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0),
                    right: attr(&attrs, "right")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0),
                    top: attr(&attrs, "top")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0),
                    bottom: attr(&attrs, "bottom")
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
                cell_xfs.push(xf_to_style(&attrs, &fonts, &fills, &borders, &numfmts, wb)?);
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
            XmlEvent::Start { name, .. } if name == "dxfs" => in_dxfs = true,
            XmlEvent::End { name } if name == "dxfs" => in_dxfs = false,
            XmlEvent::Start { name, .. } if in_dxfs && name == "dxf" => {
                in_dxf = true;
                cur_dxf = CfDxf::default();
            }
            XmlEvent::End { name } if in_dxf && name == "dxf" => {
                in_dxf = false;
                dxfs.push(cur_dxf);
            }
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if in_dxf && name == "color" =>
            {
                cur_dxf.font = Some(parse_color(&attrs, Some(theme)));
            }
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if in_dxf && (name == "fgColor" || name == "fgcolor") =>
            {
                cur_dxf.fill = Some(parse_color(&attrs, Some(theme)));
            }
            _ => {}
        }
    }
    if cell_xfs.is_empty() {
        cell_xfs.push(Style::default());
    }
    Ok(StyleTable { cell_xfs, dxfs })
}

fn xf_to_style(
    attrs: &[(String, String)],
    fonts: &[Font],
    fills: &[Fill],
    borders: &[Border],
    numfmts: &HashMap<u32, String>,
    wb: &mut Workbook,
) -> Result<Style, CoreError> {
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
    style.num_fmt = match numfmts.get(&num_id) {
        Some(code) => wb.intern_num_fmt(code)?,
        None => NumFmtId::new(num_id),
    };
    Ok(style)
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

fn load_theme(
    package: &OpcPackage,
    rels: &[Relationship],
    warnings: &mut FileWarnings,
) -> Result<Theme, CoreError> {
    let Some(rel) = rels.iter().find(|r| r.rel_type == REL_THEME) else {
        return Ok(Theme::default());
    };
    let Some(part) = package.part(&rel.target) else {
        warnings.push(
            "xlsx.part",
            "theme relationship is dangling",
            Some(rel.target.clone()),
        );
        return Ok(Theme::default());
    };
    let mut r = XmlReader::new(&part.bytes);
    let mut scheme = Vec::new();
    let mut in_scheme = false;
    while let Some(ev) = r.next()? {
        match ev {
            XmlEvent::Start { name, .. } if name == "clrScheme" => in_scheme = true,
            XmlEvent::End { name } if name == "clrScheme" => in_scheme = false,
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if in_scheme && (name == "srgbClr" || name == "sysClr") =>
            {
                let value = if name == "sysClr" {
                    attr(&attrs, "lastClr").or_else(|| attr(&attrs, "val"))
                } else {
                    attr(&attrs, "val")
                };
                if let Some(val) = value {
                    scheme.push(rgb_from_hex(val));
                }
            }
            _ => {}
        }
    }
    Ok(Theme { scheme })
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

#[derive(Clone, Copy)]
enum FragmentKind {
    AutoFilter,
    Print,
    ConditionalFormatting,
    DataValidations,
    Sparkline,
}

struct OpenFragment {
    kind: FragmentKind,
    name: String,
    start: usize,
}

fn fragment_kind(name: &str) -> Option<FragmentKind> {
    match name {
        "autoFilter" => Some(FragmentKind::AutoFilter),
        "pageSetup" | "pageMargins" | "printOptions" | "headerFooter" | "rowBreaks"
        | "colBreaks" => Some(FragmentKind::Print),
        "conditionalFormatting" => Some(FragmentKind::ConditionalFormatting),
        "dataValidations" => Some(FragmentKind::DataValidations),
        "sparklineGroups" => Some(FragmentKind::Sparkline),
        _ => None,
    }
}

fn store_fragment(extra: &mut WorksheetExtras, kind: FragmentKind, bytes: Vec<u8>) {
    match kind {
        FragmentKind::AutoFilter => extra.autofilter_xml = bytes,
        FragmentKind::Print => extra.print_xml.push(bytes),
        FragmentKind::ConditionalFormatting => extra.conditional_formatting_xml.push(bytes),
        FragmentKind::DataValidations => extra.data_validations_xml.push(bytes),
        FragmentKind::Sparkline => extra.sparkline_xml.push(bytes),
    }
}

fn capture_fragment(
    extra: &mut WorksheetExtras,
    open: &mut Vec<OpenFragment>,
    event: &XmlEvent,
    span: std::ops::Range<usize>,
    source: &[u8],
) -> Result<(), CoreError> {
    match event {
        XmlEvent::Start { name, .. } => {
            if let Some(kind) = fragment_kind(name) {
                open.push(OpenFragment {
                    kind,
                    name: name.clone(),
                    start: span.start,
                });
            }
        }
        XmlEvent::Empty { name, .. } => {
            if let Some(kind) = fragment_kind(name) {
                let bytes = source
                    .get(span)
                    .ok_or_else(|| error::xlsx_xml("XML event span is outside its part"))?;
                store_fragment(extra, kind, bytes.to_vec());
            }
        }
        XmlEvent::End { name } => {
            if let Some(index) = open.iter().rposition(|fragment| fragment.name == *name) {
                let fragment = open.remove(index);
                let bytes = source
                    .get(fragment.start..span.end)
                    .ok_or_else(|| error::xlsx_xml("XML fragment span is outside its part"))?;
                store_fragment(extra, fragment.kind, bytes.to_vec());
            }
        }
        XmlEvent::Text(_) => {}
    }
    Ok(())
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
    let mut open_fragments = Vec::new();
    let mut af_parser = super::data::AutoFilterParser::default();

    while let Some(ev) = r.next()? {
        capture_fragment(
            &mut extra,
            &mut open_fragments,
            &ev,
            r.last_span(),
            &part.bytes,
        )?;
        af_parser.feed(&ev, &styles.dxfs);
        match ev {
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if name == "tabColor" =>
            {
                wb.set_tab_color(id, Some(parse_color(&attrs, None)))?;
            }
            XmlEvent::Start { name, .. } if name == "sheetData" => in_sheet_data = true,
            XmlEvent::End { name } if name == "sheetData" => in_sheet_data = false,
            XmlEvent::Start { name, attrs } | XmlEvent::Empty { name, attrs }
                if in_sheet_data && name == "row" =>
            {
                if let Some(ridx) = attr(&attrs, "r").and_then(|s| s.parse::<u32>().ok()) {
                    if ridx == 0 || ridx > MAX_ROWS {
                        return Err(error::xlsx_limit(format!(
                            "worksheet row {ridx} is outside 1..={MAX_ROWS}"
                        )));
                    }
                    let row = ridx - 1;
                    if attr(&attrs, "hidden").is_some_and(truthy) {
                        wb.set_row_hidden(id, row, true)?;
                    }
                    if let Some(level) =
                        attr(&attrs, "outlineLevel").and_then(|s| s.parse::<u8>().ok())
                        && level > 0
                    {
                        let _ = wb.set_row_outline_level(id, row, level);
                    }
                    if attr(&attrs, "collapsed").is_some_and(truthy) {
                        let _ = wb.set_row_collapsed(id, row, true);
                    }
                    if let Some(ht) = attr(&attrs, "ht")
                        .and_then(|s| s.parse::<f64>().ok())
                        .filter(|height| height.is_finite() && *height > 0.0)
                    {
                        let px = (ht * 96.0 / 72.0).round().max(1.0) as u32;
                        wb.set_row_height(id, row, px)?;
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
            XmlEvent::Empty { name, attrs } if in_sheet_data && name == "c" => {
                let empty_ref = attr(&attrs, "r").unwrap_or("");
                let empty_type = attr(&attrs, "t").unwrap_or("n");
                let empty_style = attr(&attrs, "s").and_then(|s| s.parse().ok());
                if let Err(e) = commit_cell(
                    wb,
                    id,
                    empty_ref,
                    empty_type,
                    empty_style,
                    "",
                    "",
                    "",
                    None,
                    "",
                    "",
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
                let min = parse_u32_attr(&attrs, "min", 1)?;
                let max = parse_u32_attr(&attrs, "max", min)?;
                if min == 0 || min > max || max > u32::from(MAX_COLS) {
                    return Err(error::xlsx_limit(format!(
                        "worksheet column range {min}..={max} is outside 1..={MAX_COLS}"
                    )));
                }
                let hidden = attr(&attrs, "hidden").is_some_and(truthy);
                let width = attr(&attrs, "width")
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|width| width.is_finite() && *width > 0.0);
                for col in min..=max {
                    let idx = u16::try_from(col - 1)
                        .map_err(|_| error::xlsx_limit("worksheet column index overflow"))?;
                    if hidden {
                        wb.set_col_hidden(id, idx, true)?;
                    }
                    if let Some(level) =
                        attr(&attrs, "outlineLevel").and_then(|s| s.parse::<u8>().ok())
                        && level > 0
                    {
                        let _ = wb.set_col_outline_level(id, idx, level);
                    }
                    if attr(&attrs, "collapsed").is_some_and(truthy) {
                        let _ = wb.set_col_collapsed(id, idx, true);
                    }
                    if let Some(w) = width {
                        let px = (w * f64::from(omacell_core::geometry::DEFAULT_COL_PX) / 8.43)
                            .round()
                            .max(1.0) as u32;
                        wb.set_col_width(id, idx, px)?;
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
                if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
                    return Err(error::xlsx_format(
                        "worksheet pane has invalid split values",
                    ));
                }
                let state = attr(&attrs, "state").unwrap_or("");
                let mut view = wb
                    .sheet(id)
                    .ok_or_else(|| error::xlsx_format("worksheet id disappeared"))?
                    .view
                    .clone();
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
                wb.set_sheet_view(id, view)?;
            }
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if name == "sheetView" =>
            {
                if let Some(z) = attr(&attrs, "zoomScale").and_then(|s| s.parse::<f64>().ok()) {
                    if z.is_finite() && z > 0.0 {
                        let mut view = wb
                            .sheet(id)
                            .ok_or_else(|| error::xlsx_format("worksheet id disappeared"))?
                            .view
                            .clone();
                        view.zoom = z / 100.0;
                        wb.set_sheet_view(id, view)?;
                    }
                }
                if attr(&attrs, "showGridLines").is_some_and(|s| !truthy(s)) {
                    let mut view = wb
                        .sheet(id)
                        .ok_or_else(|| error::xlsx_format("worksheet id disappeared"))?
                        .view
                        .clone();
                    view.gridlines = false;
                    wb.set_sheet_view(id, view)?;
                }
                if attr(&attrs, "showFormulas").is_some_and(truthy) {
                    let mut view = wb
                        .sheet(id)
                        .ok_or_else(|| error::xlsx_format("worksheet id disappeared"))?
                        .view
                        .clone();
                    view.show_formulas = true;
                    wb.set_sheet_view(id, view)?;
                }
                if let Some(cell) =
                    attr(&attrs, "topLeftCell").and_then(|value| parse_a1_cell(value).ok())
                {
                    let mut view = wb
                        .sheet(id)
                        .ok_or_else(|| error::xlsx_format("worksheet id disappeared"))?
                        .view
                        .clone();
                    view.scroll_row = cell.row;
                    view.scroll_col = cell.col;
                    wb.set_sheet_view(id, view)?;
                }
            }
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if name == "selection" =>
            {
                if let Some(sq) = attr(&attrs, "sqref").and_then(|s| s.split_whitespace().next())
                    && let Ok(parsed) = parse_a1(sq)
                {
                    let mut view = wb
                        .sheet(id)
                        .ok_or_else(|| error::xlsx_format("worksheet id disappeared"))?
                        .view
                        .clone();
                    view.selection = match parsed.kind {
                        RefKind::Range(rg) => rg,
                        RefKind::Cell(c) => RangeRef::from_corners(c, c),
                    };
                    wb.set_sheet_view(id, view)?;
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
                let mut allow = omacell_core::sheet::ProtectionAllow::default();
                for (name, target) in [
                    ("selectLockedCells", &mut allow.select_locked),
                    ("selectUnlockedCells", &mut allow.select_unlocked),
                    ("formatCells", &mut allow.format_cells),
                    ("insertRows", &mut allow.insert_rows),
                    ("insertColumns", &mut allow.insert_cols),
                    ("sort", &mut allow.sort),
                    ("autoFilter", &mut allow.auto_filter),
                ] {
                    if let Some(value) = attr(&attrs, name) {
                        *target = !truthy(value);
                    }
                }
                wb.set_sheet_protection(
                    id,
                    ProtectionState {
                        enabled: true,
                        password: attr(&attrs, "hashValue")
                            .or_else(|| attr(&attrs, "password"))
                            .map(|s| s.as_bytes().to_vec()),
                        allow,
                        protected_ranges: Vec::new(),
                    },
                )?;
            }
            XmlEvent::Empty { name, attrs } if name == "protectedRange" => {
                let mut protection = wb
                    .sheet(id)
                    .ok_or_else(|| error::xlsx_format("worksheet id disappeared"))?
                    .protection
                    .clone();
                let ranges = attr(&attrs, "sqref")
                    .unwrap_or_default()
                    .split_whitespace()
                    .filter_map(|value| parse_a1(value).ok())
                    .map(|parsed| match parsed.kind {
                        RefKind::Cell(cell) => RangeRef::from_corners(cell, cell),
                        RefKind::Range(range) => range,
                    })
                    .collect::<Vec<_>>();
                if !ranges.is_empty() {
                    protection.protected_ranges.push(ProtectedRange {
                        name: attr(&attrs, "name").unwrap_or_default().to_string(),
                        ranges,
                        password: attr(&attrs, "password").map(|value| value.as_bytes().to_vec()),
                    });
                    wb.set_sheet_protection(id, protection)?;
                }
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
            _ => {}
        }
    }
    if !open_fragments.is_empty() {
        return Err(error::xlsx_xml(
            "worksheet ended inside a preserved XML fragment",
        ));
    }
    wb.set_sheet_merges(id, merges)?;
    if let Some(filter) = af_parser.take() {
        omacell_core::filter::restore_filter(wb, id, &filter)?;
    }
    Ok(extra)
}

fn parse_u32_attr(attrs: &[(String, String)], name: &str, default: u32) -> Result<u32, CoreError> {
    match attr(attrs, name) {
        Some(value) => value
            .parse()
            .map_err(|_| error::xlsx_format(format!("invalid {name}={value:?}"))),
        None => Ok(default),
    }
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
        set_formula_cached_value(wb, sheet, cell, t, v, inline, sst, warnings, part)?;
        apply_style(wb, sheet, cell, style_idx, styles)?;
        return Ok(());
    }

    match t {
        "s" => {
            let Ok(idx) = v.trim().parse::<usize>() else {
                warnings.push(
                    "xlsx.shared-string",
                    format!("invalid shared string index {v:?} at {r}"),
                    Some(part.into()),
                );
                apply_style(wb, sheet, cell, style_idx, styles)?;
                return Ok(());
            };
            if let Some((text, runs)) = sst.0.get(idx) {
                if runs.is_empty() {
                    wb.set_text(sheet, cell.row, cell.col, text)?;
                } else {
                    wb.set_rich_text(sheet, cell.row, cell.col, text, runs.clone())?;
                }
            } else {
                warnings.push(
                    "xlsx.shared-string",
                    format!("shared string index {idx} at {r} is out of range"),
                    Some(part.into()),
                );
            }
        }
        "inlineStr" | "str" => {
            let text = if t == "str" { v } else { inline };
            if !text.is_empty() {
                wb.set_text(sheet, cell.row, cell.col, &decode_ooxml_text(text))?;
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
                if let Some(n) = trimmed.parse::<f64>().ok().filter(|n| n.is_finite()) {
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

#[allow(clippy::too_many_arguments)]
fn set_formula_cached_value(
    wb: &mut Workbook,
    sheet: omacell_core::addr::SheetId,
    cell: CellRef,
    cell_type: &str,
    value: &str,
    inline: &str,
    sst: &Sst,
    warnings: &mut FileWarnings,
    part: &str,
) -> Result<(), CoreError> {
    if value.is_empty() && inline.is_empty() {
        return Ok(());
    }
    let mut slot = wb
        .get(sheet, cell.row, cell.col)?
        .copied()
        .unwrap_or_else(CellSlot::empty);
    let mut held_text = None;
    slot.value = match cell_type {
        "b" => Value::Bool(truthy(value) || value == "1"),
        "e" => Value::Error(ErrorKind::from_display(value.trim()).unwrap_or(ErrorKind::Value)),
        "str" | "inlineStr" => {
            let text = if cell_type == "str" { value } else { inline };
            let decoded = decode_ooxml_text(text);
            let id = wb.intern_text(&decoded);
            held_text = Some(id);
            Value::Text(id)
        }
        "s" => {
            let Some(index) = value.trim().parse::<usize>().ok() else {
                warnings.push(
                    "xlsx.shared-string",
                    format!("invalid cached shared-string index {value:?}"),
                    Some(part.into()),
                );
                return Ok(());
            };
            let Some((text, _)) = sst.0.get(index) else {
                warnings.push(
                    "xlsx.shared-string",
                    format!("cached shared-string index {index} is out of range"),
                    Some(part.into()),
                );
                return Ok(());
            };
            let id = wb.intern_text(text);
            held_text = Some(id);
            Value::Text(id)
        }
        "d" => parse_iso_date(value, wb.settings().date_system)
            .map(Value::Number)
            .unwrap_or(Value::Empty),
        _ => match value.trim().parse::<f64>() {
            Ok(number) if number.is_finite() => Value::Number(number),
            _ => {
                warnings.push(
                    "xlsx.cell",
                    format!("invalid numeric cached value {value:?}"),
                    Some(part.into()),
                );
                Value::Empty
            }
        },
    };
    wb.set_slot(sheet, cell.row, cell.col, slot)?;
    if let Some(id) = held_text {
        wb.release_text(id);
    }
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
        let mut banded_rows = true;
        let mut banded_cols = false;
        let mut cols: Vec<(String, Option<String>)> = Vec::new();
        let mut style_name = String::from("TableStyleMedium2");
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
                    cols.push((
                        attr(&attrs, "name").unwrap_or("Column").to_string(),
                        attr(&attrs, "totalsRowFunction").map(ToOwned::to_owned),
                    ));
                }
                XmlEvent::Empty { name: n, attrs } | XmlEvent::Start { name: n, attrs }
                    if n == "tableStyleInfo" =>
                {
                    if let Some(name) = attr(&attrs, "name") {
                        style_name = name.to_string();
                    }
                    banded_rows = attr(&attrs, "showRowStripes").is_none_or(truthy);
                    banded_cols = attr(&attrs, "showColumnStripes").is_some_and(truthy);
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
            table.banded_rows = banded_rows;
            table.banded_cols = banded_cols;
            table.style_name = style_name;
            if !cols.is_empty() {
                table.columns = cols
                    .into_iter()
                    .map(|(name, totals_fn)| TableColumn { name, totals_fn })
                    .collect();
            }
            if let Err(e) = wb.add_table(table) {
                warnings.push("xlsx.table", e.message, Some(rel.target.clone()));
            }
        }
    }
    Ok(())
}

fn load_persons(
    package: &OpcPackage,
    rels: &[Relationship],
    warnings: &mut FileWarnings,
) -> Result<HashMap<String, String>, CoreError> {
    let mut persons = HashMap::new();
    for rel in rels.iter().filter(|rel| rel.rel_type == REL_PERSON) {
        let Some(part) = package.part(&rel.target) else {
            warnings.push("xlsx.part", "person part missing", Some(rel.target.clone()));
            continue;
        };
        let mut reader = XmlReader::new(&part.bytes);
        while let Some(event) = reader.next()? {
            if let XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs } = event
                && name.eq_ignore_ascii_case("person")
                && let Some(id) = attr(&attrs, "id")
            {
                let display = attr(&attrs, "displayName")
                    .or_else(|| attr(&attrs, "userId"))
                    .unwrap_or(id);
                persons.insert(id.to_string(), display.to_string());
            }
        }
    }
    Ok(persons)
}

fn load_comments(
    wb: &mut Workbook,
    sheet: omacell_core::addr::SheetId,
    package: &OpcPackage,
    rels: &[Relationship],
    persons: &HashMap<String, String>,
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
                        let author = authors
                            .get(comment_author)
                            .filter(|author| !author.is_empty())
                            .cloned();
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
    load_threaded_comments(wb, sheet, package, rels, persons, warnings)?;
    Ok(())
}

#[derive(Clone)]
struct RawThreadComment {
    id: String,
    parent: Option<String>,
    cell_ref: String,
    person: String,
    text: String,
    resolved: bool,
}

fn load_threaded_comments(
    wb: &mut Workbook,
    sheet: omacell_core::addr::SheetId,
    package: &OpcPackage,
    rels: &[Relationship],
    persons: &HashMap<String, String>,
    warnings: &mut FileWarnings,
) -> Result<(), CoreError> {
    for rel in rels
        .iter()
        .filter(|rel| rel.rel_type == REL_THREADED_COMMENTS)
    {
        let Some(part) = package.part(&rel.target) else {
            warnings.push(
                "xlsx.part",
                "threaded comments part missing",
                Some(rel.target.clone()),
            );
            continue;
        };
        let mut reader = XmlReader::new(&part.bytes);
        let mut current: Option<RawThreadComment> = None;
        let mut in_text = false;
        let mut records = BTreeMap::new();
        while let Some(event) = reader.next()? {
            match event {
                XmlEvent::Start { name, attrs } if name == "threadedComment" => {
                    current = Some(RawThreadComment {
                        id: attr(&attrs, "id").unwrap_or_default().to_string(),
                        parent: attr(&attrs, "parentId").map(ToOwned::to_owned),
                        cell_ref: attr(&attrs, "ref").unwrap_or_default().to_string(),
                        person: attr(&attrs, "personId").unwrap_or_default().to_string(),
                        text: String::new(),
                        resolved: attr(&attrs, "done").is_some_and(truthy),
                    });
                }
                XmlEvent::End { name } if name == "threadedComment" => {
                    if let Some(record) = current.take()
                        && !record.id.is_empty()
                    {
                        records.insert(record.id.clone(), record);
                    }
                }
                XmlEvent::Start { name, .. } if name == "text" => in_text = true,
                XmlEvent::End { name } if name == "text" => in_text = false,
                XmlEvent::Text(text) if in_text => {
                    if let Some(record) = &mut current {
                        record.text.push_str(&text);
                    }
                }
                _ => {}
            }
        }
        let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for record in records.values() {
            if let Some(parent) = &record.parent {
                children
                    .entry(parent.clone())
                    .or_default()
                    .push(record.id.clone());
            }
        }
        for record in records.values().filter(|record| record.parent.is_none()) {
            let Ok(cell) = parse_a1_cell(&record.cell_ref) else {
                warnings.push(
                    "xlsx.comment",
                    format!("invalid threaded comment ref {:?}", record.cell_ref),
                    Some(rel.target.clone()),
                );
                continue;
            };
            let comment = build_thread_comment(&record.id, &records, &children, persons, 0)?;
            wb.set_comment(sheet, cell.row, cell.col, Some(comment))?;
        }
    }
    Ok(())
}

fn build_thread_comment(
    id: &str,
    records: &BTreeMap<String, RawThreadComment>,
    children: &BTreeMap<String, Vec<String>>,
    persons: &HashMap<String, String>,
    depth: usize,
) -> Result<omacell_core::sheet::Comment, CoreError> {
    if depth >= 64 {
        return Err(error::xlsx_limit("threaded comment nesting exceeds 64"));
    }
    let record = records
        .get(id)
        .ok_or_else(|| error::xlsx_format("threaded comment parent is missing"))?;
    let replies = children
        .get(id)
        .into_iter()
        .flatten()
        .map(|child| build_thread_comment(child, records, children, persons, depth + 1))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(omacell_core::sheet::Comment {
        author: persons
            .get(&record.person)
            .cloned()
            .unwrap_or_else(|| record.person.clone()),
        text: record.text.clone(),
        replies,
        resolved: record.resolved,
    })
}

fn load_charts(
    wb: &mut Workbook,
    sheet: omacell_core::addr::SheetId,
    package: &OpcPackage,
    rels: &[Relationship],
) {
    for drawing in rels.iter().filter(|r| r.rel_type == REL_DRAWING) {
        let anchors = package
            .part(&drawing.target)
            .map(|part| super::drawing::parse_drawing_anchors(&part.bytes))
            .unwrap_or_default();
        let Ok(drels) = package.rels_for(&drawing.target) else {
            continue;
        };
        for crel in drels.iter().filter(|r| r.rel_type == REL_CHART) {
            let Some(part) = package.part(&crel.target) else {
                continue;
            };
            let anchor = anchors.get(&crel.id).copied().unwrap_or_default();
            if let Some(chart) = super::drawing::parse_chart_part(&part.bytes, wb, sheet, anchor) {
                let _ = wb.add_chart(chart);
            }
        }
    }
}

fn load_omacell_parts(wb: &mut Workbook, package: &OpcPackage) {
    for (name, part) in &package.parts {
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("xl/omacell/") && !lower.ends_with(".rels") {
            wb.custom_parts.insert(name.clone(), part.bytes.clone());
        }
    }
}
