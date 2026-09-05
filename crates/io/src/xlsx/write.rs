//! Regenerate modeled OPC parts and re-emit preserved L3 bytes.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{Cursor, Write};

use indexmap::IndexMap;
use omacell_core::addr::{RangeRef, SheetSpec, col_to_letters, quote_sheet_name};
use omacell_core::condfmt::CfDxf;
use omacell_core::error::CoreError;
use omacell_core::geometry::DEFAULT_COL_PX;
use omacell_core::intern::{Interners, RichTextRun};
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use omacell_core::pivot::PivotTable;
use omacell_core::sheet::{ArrayFormula, ProtectionAllow, ProtectionState, Sheet, SheetVisibility};
use omacell_core::storage::CellSlot;
use omacell_core::style::{
    BorderStyle, Color, Fill, Font, GradientKind, PatternType, Style, StyleId, Underline,
};
use omacell_core::tables::Table;
use omacell_core::value::{StrId, Value};
use omacell_core::workbook::{CalcMode, DateSystem, Workbook, WorkbookProtectionState};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use super::WorksheetExtras;
use super::drawing;
use super::opc::{
    MAX_ENTRY_BYTES, MAX_PACKAGE_BYTES, MAX_UNCOMPRESSED_TOTAL, MAX_ZIP_ENTRIES, OpcPackage,
    relative_target, sanitize_path,
};
use super::print as xlsx_print;
use super::{XlsxDocument, split_pixels_to_twips, xml};
use crate::error;

const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const NS_X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const NS_XM: &str = "http://schemas.microsoft.com/office/excel/2006/main";
const NS_XR: &str = "http://schemas.microsoft.com/office/spreadsheetml/2014/revision";
const NS_PKG: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const NS_CT: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const EXT_CONDITIONAL_FORMATTING: &str = "{78C0D931-6437-407D-A8EE-F0AAD7539E65}";
const EXT_DATA_VALIDATIONS: &str = "{CCE6A557-97BC-4B89-ADB6-D9C93CAAB3DF}";
const EXT_SPARKLINES: &str = "{05C60535-1F16-4FD2-B633-F4F36F0B64E0}";
const REL_OFFICE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
const REL_WS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";
const REL_SST: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings";
const REL_STYLES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
const REL_TABLE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/table";
const REL_PIVOT_CACHE_DEF: &str = super::pivot::REL_PIVOT_CACHE_DEF;
const REL_PIVOT_TABLE: &str = super::pivot::REL_PIVOT_TABLE;
const REL_COMMENTS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments";
const REL_THREADED_COMMENTS: &str =
    "http://schemas.microsoft.com/office/2017/10/relationships/threadedComment";
const REL_PERSON: &str = "http://schemas.microsoft.com/office/2017/10/relationships/person";
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
const CT_THREADED_CMT: &str = "application/vnd.ms-excel.threadedcomments+xml";
const CT_PERSON: &str = "application/vnd.ms-excel.person+xml";
const CT_VML: &str = "application/vnd.openxmlformats-officedocument.vmlDrawing";

/// Encode `doc` as `.xlsx` bytes (modeled parts regenerated, L3 copied).
pub fn save_bytes(doc: &XlsxDocument) -> Result<Vec<u8>, CoreError> {
    encode(&doc.workbook, &doc.extras, Some(&doc.package))
}

/// Encode a workbook with no preserved package (new file).
pub fn save_workbook_bytes(wb: &Workbook) -> Result<Vec<u8>, CoreError> {
    encode(wb, &HashMap::new(), None)
}

fn pivots_for_write(
    wb: &Workbook,
    package: Option<&OpcPackage>,
) -> Result<Vec<PivotTable>, CoreError> {
    let mut pivots: Vec<PivotTable> = wb.pivots().iter().cloned().collect();
    let mut used_ids: BTreeSet<u32> = pivots
        .iter()
        .filter_map(|pivot| pivot.ooxml_cache_id)
        .collect();
    let mut used_parts: BTreeSet<String> = package
        .into_iter()
        .flat_map(|package| package.parts.keys())
        .map(|name| name.replace('\\', "/").to_ascii_lowercase())
        .collect();
    for pivot in &pivots {
        if let Some(name) = &pivot.ooxml_cache_def {
            used_parts.insert(name.replace('\\', "/").to_ascii_lowercase());
        }
        if let Some(name) = &pivot.ooxml_table {
            used_parts.insert(name.replace('\\', "/").to_ascii_lowercase());
        }
    }

    let mut next_id = 1u32;
    let mut next_cache_part = 1u32;
    let mut next_table_part = 1u32;
    for pivot in &mut pivots {
        if pivot.ooxml_cache_id.is_none() {
            pivot.ooxml_cache_id = Some(take_unused_id(&mut used_ids, &mut next_id)?);
        }
        if pivot.ooxml_cache_def.is_none() {
            pivot.ooxml_cache_def = Some(take_unused_cache_part(
                &mut used_parts,
                &mut next_cache_part,
            )?);
        }
        if pivot.ooxml_table.is_none() {
            pivot.ooxml_table = Some(take_unused_table_part(
                &mut used_parts,
                &mut next_table_part,
            )?);
        }
    }

    // A cacheId may be shared only when it names the same cache definition.
    // Malformed/imported identifiers must not make one generated relationship
    // silently suppress another cache part.
    let mut owners: BTreeMap<u32, String> = BTreeMap::new();
    for pivot in &mut pivots {
        let id = super::pivot::cache_id_of(pivot);
        let definition = pivot.ooxml_cache_def.clone().unwrap_or_default();
        if owners
            .get(&id)
            .is_some_and(|owner| !owner.eq_ignore_ascii_case(&definition))
        {
            pivot.ooxml_cache_id = Some(take_unused_id(&mut used_ids, &mut next_id)?);
        }
        owners.insert(super::pivot::cache_id_of(pivot), definition);
    }
    Ok(pivots)
}

fn take_unused_id(used: &mut BTreeSet<u32>, next: &mut u32) -> Result<u32, CoreError> {
    loop {
        let candidate = *next;
        *next = next
            .checked_add(1)
            .ok_or_else(|| error::xlsx_write("pivot cache id space is exhausted"))?;
        if used.insert(candidate) {
            return Ok(candidate);
        }
    }
}

fn take_unused_cache_part(
    used: &mut BTreeSet<String>,
    next: &mut u32,
) -> Result<String, CoreError> {
    loop {
        let number = *next;
        *next = next
            .checked_add(1)
            .ok_or_else(|| error::xlsx_write("pivot cache part id space is exhausted"))?;
        let definition = format!("xl/pivotCache/pivotCacheDefinition{number}.xml");
        let records = format!("xl/pivotCache/pivotCacheRecords{number}.xml");
        let rels = format!("xl/pivotCache/_rels/pivotCacheDefinition{number}.xml.rels");
        if [&definition, &records, &rels]
            .iter()
            .all(|name| !used.contains(&name.to_ascii_lowercase()))
        {
            used.insert(definition.to_ascii_lowercase());
            used.insert(records.to_ascii_lowercase());
            used.insert(rels.to_ascii_lowercase());
            return Ok(definition);
        }
    }
}

fn take_unused_table_part(
    used: &mut BTreeSet<String>,
    next: &mut u32,
) -> Result<String, CoreError> {
    loop {
        let number = *next;
        *next = next
            .checked_add(1)
            .ok_or_else(|| error::xlsx_write("pivot table part id space is exhausted"))?;
        let table = format!("xl/pivotTables/pivotTable{number}.xml");
        let rels = format!("xl/pivotTables/_rels/pivotTable{number}.xml.rels");
        if [&table, &rels]
            .iter()
            .all(|name| !used.contains(&name.to_ascii_lowercase()))
        {
            used.insert(table.to_ascii_lowercase());
            used.insert(rels.to_ascii_lowercase());
            return Ok(table);
        }
    }
}

pub(crate) fn encode(
    wb: &Workbook,
    extras: &HashMap<String, WorksheetExtras>,
    package: Option<&OpcPackage>,
) -> Result<Vec<u8>, CoreError> {
    let intern = wb.intern();
    let sheets: Vec<&Sheet> = wb.sheets().collect();
    let pivots = pivots_for_write(wb, package)?;
    let persons = threaded_persons(&sheets);
    if sheets.is_empty() {
        return Err(error::xlsx_write("workbook has no sheets"));
    }

    if !sheets.iter().any(|sheet| sheet.visibility.is_visible()) {
        return Err(error::xlsx_write("workbook has no visible sheets"));
    }

    let mut sst: IndexMap<StrId, u32> = IndexMap::new();
    let mut sst_count = 0u64;
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
                && slot.formula.is_none_or(|formula| {
                    intern
                        .formulas
                        .get(formula)
                        .is_some_and(super::ai_formula::is_ai_formula)
                })
            {
                sst_count = sst_count.saturating_add(1);
                if !sst.contains_key(&id) {
                    let i = u32::try_from(sst.len())
                        .map_err(|_| error::xlsx_write("shared string table is too large"))?;
                    sst.insert(id, i);
                }
            }
            if let Some(style) = intern.styles.get(slot.style) {
                validate_style(style)?;
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

    parts.insert(
        "xl/sharedStrings.xml".into(),
        sst_xml(&sst, intern, sst_count)?,
    );
    overrides.push(("/xl/sharedStrings.xml".into(), CT_SST.into()));
    let mut dxfs: Vec<CfDxf> = Vec::new();
    for sheet in &sheets {
        for rule in &sheet.cond_formats {
            if (rule.dxf.fill.is_some() || rule.dxf.font.is_some())
                && !dxfs.iter().any(|d| d == &rule.dxf)
            {
                dxfs.push(rule.dxf);
            }
        }
        if let Some(filter) = &sheet.autofilter {
            for column in &filter.columns {
                if let omacell_core::filter::FilterCriteria::Color { fill, argb } = &column.criteria
                {
                    let dxf = if *fill {
                        CfDxf {
                            fill: Some(Color::Rgb { argb: *argb }),
                            font: None,
                        }
                    } else {
                        CfDxf {
                            fill: None,
                            font: Some(Color::Rgb { argb: *argb }),
                        }
                    };
                    if !dxfs.iter().any(|candidate| candidate == &dxf) {
                        dxfs.push(dxf);
                    }
                }
            }
        }
    }
    parts.insert(
        "xl/styles.xml".into(),
        styles_xml(wb, &fonts, &fills, &borders, &xfs, &dxfs),
    );
    overrides.push(("/xl/styles.xml".into(), CT_STY.into()));

    let original_wb_rels = if let Some((package, workbook)) = package.and_then(|package| {
        package
            .workbook_part()
            .ok()
            .map(|workbook| (package, workbook))
    }) {
        package.rels_for(&workbook.name)?
    } else {
        Vec::new()
    };
    let mut workbook_relationship_ids =
        RelationshipIdAllocator::new(&original_wb_rels, "source workbook", |relationship| {
            !is_regenerated_workbook_relationship(relationship)
        })?;
    let mut wb_rels: Vec<(String, String, String, bool)> = Vec::new();
    let mut sheet_rids = Vec::new();
    let mut drawing_names = drawing::PartNameAllocator::new(package);
    let mut vml_shape_ids = VmlShapeIdAllocator::new(package);
    for (i, sheet) in sheets.iter().enumerate() {
        let r = workbook_relationship_ids.next()?;
        let target = format!("worksheets/sheet{}.xml", i + 1);
        wb_rels.push((r.clone(), REL_WS.into(), target.clone(), false));
        sheet_rids.push(r);
        let sheet_extras =
            extras_for_sheet(extras, package, &sheet.name, sheet.id.index() as usize)?;
        let (sheet_xml, sheet_rels, extra_parts) = worksheet_xml(
            wb,
            sheet,
            sheet_extras,
            &sst,
            &xf_index,
            intern,
            i,
            package,
            &persons,
            &dxfs,
            &pivots,
            &mut drawing_names,
            &mut vml_shape_ids,
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
            if !ct.is_empty() {
                overrides.push((format!("/{name}"), ct));
            }
            parts.insert(name, bytes);
        }
    }
    let sst_rid = workbook_relationship_ids.next()?;
    wb_rels.push((sst_rid, REL_SST.into(), "sharedStrings.xml".into(), false));
    let sty_rid = workbook_relationship_ids.next()?;
    wb_rels.push((sty_rid, REL_STYLES.into(), "styles.xml".into(), false));
    if !persons.is_empty() {
        let person_rid = workbook_relationship_ids.next()?;
        wb_rels.push((
            person_rid,
            REL_PERSON.into(),
            "persons/person.xml".into(),
            false,
        ));
        parts.insert("xl/persons/person.xml".into(), persons_xml(&persons));
        overrides.push(("/xl/persons/person.xml".into(), CT_PERSON.into()));
    }

    let mut pivot_caches: Vec<(u32, String)> = Vec::new();
    let mut cache_groups: BTreeMap<u32, Vec<&PivotTable>> = BTreeMap::new();
    for pivot in &pivots {
        cache_groups
            .entry(super::pivot::cache_id_of(pivot))
            .or_default()
            .push(pivot);
    }
    for group in cache_groups.into_values() {
        let Some(first) = group.first().copied() else {
            continue;
        };
        let pivot = group
            .iter()
            .copied()
            .find(|pivot| pivot.ooxml_dirty)
            .unwrap_or(first);
        let cache = if let Some(pkg) = package {
            super::pivot::preserved_cache_parts(pkg, pivot)
        } else {
            None
        };
        let cache = match cache {
            Some(parts) => parts,
            None => super::pivot::cache_parts(wb, pivot)?,
        };
        let r = workbook_relationship_ids.next()?;
        wb_rels.push((
            r.clone(),
            REL_PIVOT_CACHE_DEF.into(),
            cache.def_target,
            false,
        ));
        pivot_caches.push((cache.cache_id, r));
        for (name, bytes, ct) in cache.parts {
            if !ct.is_empty() {
                overrides.push((format!("/{name}"), ct));
            }
            parts.insert(name, bytes);
        }
    }

    for rel in &original_wb_rels {
        if is_regenerated_workbook_relationship(rel) {
            continue;
        }
        if is_rewritten(&rel.target) {
            let custom_is_present = rel.target.to_ascii_lowercase().starts_with("xl/omacell/")
                && wb
                    .custom_parts
                    .keys()
                    .any(|name| name.eq_ignore_ascii_case(&rel.target));
            if !custom_is_present {
                continue;
            }
        }
        let target = if rel.external {
            rel.target.clone()
        } else {
            relative_target("xl/workbook.xml", &rel.target)
        };
        wb_rels.push((rel.id.clone(), rel.rel_type.clone(), target, rel.external));
    }

    parts.insert(
        "xl/workbook.xml".into(),
        workbook_xml(wb, intern, &sheets, &sheet_rids, &pivot_caches, package)?,
    );
    let workbook_content_type = package
        .and_then(|pkg| pkg.workbook_part().ok())
        .and_then(|part| part.content_type.clone())
        .unwrap_or_else(|| CT_WB.into());
    overrides.push(("/xl/workbook.xml".into(), workbook_content_type));
    parts.insert("xl/_rels/workbook.xml.rels".into(), rels_xml(&wb_rels));

    if let Some(bytes) = super::ai_formula::encode(wb)? {
        parts.insert(super::ai_formula::PART.into(), bytes);
        overrides.push((
            format!("/{}", super::ai_formula::PART),
            "application/json".into(),
        ));
    }

    for (name, bytes) in &wb.custom_parts {
        if name.eq_ignore_ascii_case(super::ai_formula::PART) {
            continue;
        }
        let name = custom_part_name(name)?;
        if contains_part(&parts, &name) {
            return Err(error::xlsx_write(format!(
                "duplicate generated OPC part {name:?}"
            )));
        }
        let content_type = package
            .and_then(|pkg| pkg.part(&name))
            .and_then(|part| part.content_type.clone())
            .unwrap_or_else(|| "application/json".into());
        parts.insert(name.clone(), bytes.clone());
        overrides.push((format!("/{name}"), content_type));
    }

    if let Some(pkg) = package {
        for (name, part) in &pkg.parts {
            if is_rewritten(name) {
                continue;
            }
            if contains_part(&parts, name) {
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
        || n.starts_with("xl/pivotcache/")
        || n.starts_with("xl/pivottables/")
        || n.starts_with("xl/comments")
        || n.starts_with("xl/threadedcomments/")
        || n.starts_with("xl/persons/")
        || n.starts_with("xl/omacell/")
}

fn zip_parts(parts: &IndexMap<String, Vec<u8>>) -> Result<Vec<u8>, CoreError> {
    if parts.len() > MAX_ZIP_ENTRIES {
        return Err(error::xlsx_write(format!(
            "output has {} entries; maximum is {MAX_ZIP_ENTRIES}",
            parts.len()
        )));
    }
    let mut total = 0u64;
    for (name, data) in parts {
        let len = data.len() as u64;
        if len > MAX_ENTRY_BYTES {
            return Err(error::xlsx_write(format!(
                "output part {name} is {len} bytes; maximum is {MAX_ENTRY_BYTES}"
            )));
        }
        total = total.saturating_add(len);
        if total > MAX_UNCOMPRESSED_TOTAL {
            return Err(error::xlsx_write(format!(
                "uncompressed output exceeds {MAX_UNCOMPRESSED_TOTAL} bytes"
            )));
        }
    }
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
    let bytes = buf.into_inner();
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(error::xlsx_write(format!(
            "compressed output is {} bytes; maximum is {MAX_PACKAGE_BYTES}",
            bytes.len()
        )));
    }
    Ok(bytes)
}

fn contains_part(parts: &IndexMap<String, Vec<u8>>, name: &str) -> bool {
    parts
        .keys()
        .any(|existing| existing.eq_ignore_ascii_case(name))
}

fn custom_part_name(name: &str) -> Result<String, CoreError> {
    let sanitized = sanitize_path(name)?;
    let normalized = sanitized.replace('\\', "/");
    if normalized != name || !normalized.to_ascii_lowercase().starts_with("xl/omacell/") {
        return Err(error::xlsx_write(format!(
            "custom part {name:?} must be below xl/omacell/"
        )));
    }
    if normalized.ends_with('/') {
        return Err(error::xlsx_write(format!(
            "custom part {name:?} must name a file"
        )));
    }
    Ok(normalized)
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

struct PreservedXmlElement {
    raw: Vec<u8>,
    attrs: Vec<(String, String)>,
}

struct PreservedXmlCapture {
    start: usize,
    attrs: Vec<(String, String)>,
    depth: u32,
}

struct RelationshipIdAllocator {
    used: HashSet<String>,
    next: u32,
}

struct VmlShapeIdAllocator {
    used: HashSet<String>,
    next: u64,
}

impl VmlShapeIdAllocator {
    fn new(package: Option<&OpcPackage>) -> Self {
        let mut used = HashSet::new();
        if let Some(package) = package {
            for part in package.parts.values().filter(|part| {
                part.name.to_ascii_lowercase().ends_with(".vml")
                    || part.content_type.as_deref() == Some(CT_VML)
            }) {
                const PREFIX: &[u8] = b"_x0000_s";
                for (offset, window) in part.bytes.windows(PREFIX.len()).enumerate() {
                    if window.eq_ignore_ascii_case(PREFIX) {
                        let digits = &part.bytes[offset + PREFIX.len()..];
                        let length = digits
                            .iter()
                            .take_while(|byte| byte.is_ascii_digit())
                            .count();
                        if length > 0 {
                            used.insert(
                                String::from_utf8_lossy(
                                    &part.bytes[offset..offset + PREFIX.len() + length],
                                )
                                .to_ascii_lowercase(),
                            );
                        }
                    }
                }
            }
        }
        Self { used, next: 1025 }
    }

    fn next(&mut self) -> Result<String, CoreError> {
        loop {
            let id = format!("_x0000_s{}", self.next);
            self.next = self
                .next
                .checked_add(1)
                .ok_or_else(|| error::xlsx_write("VML shape id space is exhausted"))?;
            if self.used.insert(id.to_ascii_lowercase()) {
                return Ok(id);
            }
        }
    }
}

fn is_regenerated_workbook_relationship(relationship: &super::opc::Relationship) -> bool {
    matches!(
        relationship.rel_type.as_str(),
        REL_WS | REL_SST | REL_STYLES | REL_PERSON | REL_PIVOT_CACHE_DEF
    )
}

impl RelationshipIdAllocator {
    fn new<F>(
        relationships: &[super::opc::Relationship],
        source: &str,
        reserve: F,
    ) -> Result<Self, CoreError>
    where
        F: Fn(&super::opc::Relationship) -> bool,
    {
        let mut seen = HashSet::new();
        let mut used = HashSet::new();
        for relationship in relationships {
            if relationship.id.is_empty() || !seen.insert(relationship.id.clone()) {
                return Err(error::xlsx_write(format!(
                    "{source} relationship ids must be non-empty and unique"
                )));
            }
            if reserve(relationship) {
                used.insert(relationship.id.clone());
            }
        }
        Ok(Self { used, next: 1 })
    }

    fn next(&mut self) -> Result<String, CoreError> {
        loop {
            let id = format!("rId{}", self.next);
            self.next = self
                .next
                .checked_add(1)
                .ok_or_else(|| error::xlsx_write("relationship id space is exhausted"))?;
            if self.used.insert(id.clone()) {
                return Ok(id);
            }
        }
    }
}

fn first_xml_element(bytes: &[u8], wanted: &str) -> Result<Option<PreservedXmlElement>, CoreError> {
    let mut reader = xml::XmlReader::new(bytes);
    let mut target: Option<PreservedXmlCapture> = None;
    while let Some(event) = reader.next()? {
        let span = reader.last_span();
        match event {
            xml::XmlEvent::Start { name, attrs } => {
                if let Some(target) = target.as_mut() {
                    target.depth += 1;
                } else if name == wanted {
                    target = Some(PreservedXmlCapture {
                        start: span.start,
                        attrs,
                        depth: 1,
                    });
                }
            }
            xml::XmlEvent::Empty { name, attrs } if target.is_none() && name == wanted => {
                let raw = bytes
                    .get(span)
                    .ok_or_else(|| error::xlsx_write("preserved XML span is outside its part"))?
                    .to_vec();
                return Ok(Some(PreservedXmlElement { raw, attrs }));
            }
            xml::XmlEvent::End { .. } => {
                if let Some(target) = target.as_mut() {
                    target.depth = target.depth.saturating_sub(1);
                    if target.depth == 0 {
                        let raw = bytes
                            .get(target.start..span.end)
                            .ok_or_else(|| {
                                error::xlsx_write("preserved XML span is outside its part")
                            })?
                            .to_vec();
                        return Ok(Some(PreservedXmlElement {
                            raw,
                            attrs: std::mem::take(&mut target.attrs),
                        }));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(None)
}

fn original_workbook_element(
    package: Option<&OpcPackage>,
    wanted: &str,
) -> Result<Option<PreservedXmlElement>, CoreError> {
    let Some(package) = package else {
        return Ok(None);
    };
    let Ok(workbook) = package.workbook_part() else {
        return Ok(None);
    };
    first_xml_element(&workbook.bytes, wanted)
}

fn push_preserved_attrs(out: &mut String, attrs: &[(String, String)], excluded: &[&str]) {
    for (name, value) in attrs {
        if !excluded.contains(&name.as_str()) {
            out.push_str(&format!(r#" {name}="{}""#, xml::escape(value)));
        }
    }
}

fn xml_truthy(value: &str) -> bool {
    value == "1" || value.eq_ignore_ascii_case("true")
}

fn workbook_protection_matches(
    attrs: &[(String, String)],
    protection: &WorkbookProtectionState,
) -> bool {
    let password = xml::attr(attrs, "workbookHashValue")
        .or_else(|| xml::attr(attrs, "workbookPassword"))
        .map(str::as_bytes);
    protection.enabled
        && protection.lock_structure == xml::attr(attrs, "lockStructure").is_some_and(xml_truthy)
        && protection.lock_windows == xml::attr(attrs, "lockWindows").is_some_and(xml_truthy)
        && protection.password.as_deref() == password
}

fn workbook_xml(
    wb: &Workbook,
    intern: &omacell_core::intern::Interners,
    sheets: &[&Sheet],
    rids: &[String],
    pivot_caches: &[(u32, String)],
    package: Option<&OpcPackage>,
) -> Result<Vec<u8>, CoreError> {
    let mut s = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="{NS}" xmlns:r="{NS_R}">"#
    );
    let original_workbook_pr = original_workbook_element(package, "workbookPr")?;
    if original_workbook_pr.is_some() || wb.settings().date_system == DateSystem::Excel1904 {
        s.push_str("<workbookPr");
        if let Some(original) = &original_workbook_pr {
            push_preserved_attrs(&mut s, &original.attrs, &["date1904"]);
        }
        if wb.settings().date_system == DateSystem::Excel1904 {
            s.push_str(r#" date1904="1""#);
        }
        s.push_str("/>");
    }
    if wb.protection().enabled {
        let original = original_workbook_element(package, "workbookProtection")?;
        if let Some(raw) = original
            .as_ref()
            .filter(|element| workbook_protection_matches(&element.attrs, wb.protection()))
        {
            s.push_str(
                std::str::from_utf8(&raw.raw)
                    .map_err(|_| error::xlsx_write("preserved workbook protection is not UTF-8"))?,
            );
        } else {
            let password = wb
                .protection()
                .password
                .as_deref()
                .map(std::str::from_utf8)
                .transpose()
                .map_err(|_| error::xlsx_write("workbook protection verifier is not UTF-8"))?
                .map(|value| format!(r#" workbookPassword="{}""#, xml::escape(value)))
                .unwrap_or_default();
            let structure = if wb.protection().lock_structure {
                r#" lockStructure="1""#
            } else {
                ""
            };
            let windows = if wb.protection().lock_windows {
                r#" lockWindows="1""#
            } else {
                ""
            };
            s.push_str(&format!(
                r#"<workbookProtection{password}{structure}{windows}/>"#
            ));
        }
    }
    let active = sheets
        .iter()
        .position(|sh| sh.id == wb.active_sheet() && sh.visibility.is_visible())
        .or_else(|| sheets.iter().position(|sh| sh.visibility.is_visible()))
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
    if let Some(external_references) = original_workbook_element(package, "externalReferences")? {
        s.push_str(
            std::str::from_utf8(&external_references.raw)
                .map_err(|_| error::xlsx_write("preserved external references are not UTF-8"))?,
        );
    }
    let names: Vec<_> = wb.names().iter().collect();
    let rewrite_print_names: Vec<bool> = sheets
        .iter()
        .map(|sheet| {
            let existing: Vec<_> = names
                .iter()
                .filter(|name| {
                    matches!(name.scope, omacell_core::names::NameScope::Sheet(id) if id == sheet.id)
                        && xlsx_print::is_print_name(&name.name)
                })
                .map(|name| {
                    let referent = match &name.referent {
                        omacell_core::names::NameReferent::Range(range) => range.to_a1(),
                        omacell_core::names::NameReferent::Formula(formula) => formula.clone(),
                        omacell_core::names::NameReferent::Constant(_) => String::new(),
                    };
                    (name.name.as_str(), referent)
                })
                .collect();
            !xlsx_print::print_names_match(
                &sheet.page_setup,
                existing.iter().map(|(name, referent)| (*name, referent.as_str())),
            )
        })
        .collect();
    let mut names_xml = String::new();
    for n in &names {
        if matches!(n.scope, omacell_core::names::NameScope::Sheet(_))
            && super::data::is_filter_database_name(&n.name)
        {
            continue;
        }
        let rewrite = match n.scope {
            omacell_core::names::NameScope::Workbook => false,
            omacell_core::names::NameScope::Sheet(id) => sheets
                .iter()
                .position(|sheet| sheet.id == id)
                .is_some_and(|index| rewrite_print_names[index]),
        };
        if rewrite && xlsx_print::is_print_name(&n.name) {
            continue;
        }
        let local = match n.scope {
            omacell_core::names::NameScope::Workbook => String::new(),
            omacell_core::names::NameScope::Sheet(id) => sheets
                .iter()
                .position(|sh| sh.id == id)
                .map(|i| format!(r#" localSheetId="{i}""#))
                .unwrap_or_default(),
        };
        let text = match &n.referent {
            omacell_core::names::NameReferent::Range(r) => defined_name_range_text(wb, *r)?,
            omacell_core::names::NameReferent::Formula(f) => super::formula::to_xlsx(f),
            omacell_core::names::NameReferent::Constant(v) => constant_name_text(intern, *v),
        };
        let comment = n
            .comment
            .as_ref()
            .map(|value| format!(r#" comment="{}""#, xml::escape(value)))
            .unwrap_or_default();
        names_xml.push_str(&format!(
            r#"<definedName name="{}"{local}{comment}>{}</definedName>"#,
            xml::escape(&n.name),
            xml::escape(&text)
        ));
    }
    for (i, sheet) in sheets.iter().enumerate() {
        if rewrite_print_names[i] {
            names_xml.push_str(&xlsx_print::print_names_xml(sheet, i));
        }
        names_xml.push_str(&filter_database_name_xml(sheet, i)?);
    }
    if !names_xml.is_empty() {
        s.push_str("<definedNames>");
        s.push_str(&names_xml);
        s.push_str("</definedNames>");
    }
    match wb.settings().calc_mode {
        CalcMode::Manual => s.push_str(r#"<calcPr calcMode="manual"/>"#),
        CalcMode::AutomaticExceptTables => s.push_str(r#"<calcPr calcMode="autoNoTable"/>"#),
        CalcMode::Automatic => {}
    }
    if !pivot_caches.is_empty() {
        s.push_str(&format!(r#"<pivotCaches count="{}">"#, pivot_caches.len()));
        for (cache_id, rid) in pivot_caches {
            s.push_str(&format!(
                r#"<pivotCache cacheId="{cache_id}" r:id="{rid}"/>"#
            ));
        }
        s.push_str("</pivotCaches>");
    }
    s.push_str("</workbook>");
    Ok(s.into_bytes())
}

fn sheet_pr_xml(
    sheet: &Sheet,
    filter_mode: bool,
    original: Option<&PreservedXmlElement>,
) -> Result<String, CoreError> {
    if original.is_none() && sheet.tab_color.is_none() && !filter_mode {
        return Ok(String::new());
    }
    let mut out = String::from("<sheetPr");
    if let Some(original) = original {
        push_preserved_attrs(&mut out, &original.attrs, &["filterMode"]);
    }
    if filter_mode {
        out.push_str(r#" filterMode="1""#);
    }
    out.push('>');
    if let Some(color) = sheet.tab_color {
        out.push_str(&color_tag("tabColor", &color));
    }
    if let Some(original) = original {
        for child in ["outlinePr", "pageSetUpPr"] {
            if let Some(element) = first_xml_element(&original.raw, child)? {
                out.push_str(std::str::from_utf8(&element.raw).map_err(|_| {
                    error::xlsx_write(format!("preserved {child} property is not UTF-8"))
                })?);
            }
        }
    }
    out.push_str("</sheetPr>");
    Ok(out)
}

fn sheet_protection_matches(attrs: &[(String, String)], protection: &ProtectionState) -> bool {
    let password = xml::attr(attrs, "hashValue")
        .or_else(|| xml::attr(attrs, "password"))
        .map(str::as_bytes);
    let mut allow = ProtectionAllow::default();
    for (name, target) in [
        ("selectLockedCells", &mut allow.select_locked),
        ("selectUnlockedCells", &mut allow.select_unlocked),
        ("formatCells", &mut allow.format_cells),
        ("insertRows", &mut allow.insert_rows),
        ("insertColumns", &mut allow.insert_cols),
        ("sort", &mut allow.sort),
        ("autoFilter", &mut allow.auto_filter),
    ] {
        if let Some(value) = xml::attr(attrs, name) {
            *target = !xml_truthy(value);
        }
    }
    protection.enabled && protection.password.as_deref() == password && protection.allow == allow
}

fn defined_name_range_text(wb: &Workbook, range: RangeRef) -> Result<String, CoreError> {
    if range.start.sheet != range.end.sheet {
        return Err(error::xlsx_write(
            "defined-name range endpoints have different sheets",
        ));
    }
    let body = range.to_a1();
    let Some(start_id) = range.start.sheet else {
        if range.sheet_end.is_some() {
            return Err(error::xlsx_write(
                "defined-name 3-D range is missing its start sheet",
            ));
        }
        return Ok(body);
    };
    let start = wb
        .sheet(start_id)
        .ok_or_else(|| error::xlsx_write("defined name has an unknown sheet id"))?;
    let prefix = if let Some(end_id) = range.sheet_end {
        let end = wb
            .sheet(end_id)
            .ok_or_else(|| error::xlsx_write("defined name has an unknown end sheet id"))?;
        SheetSpec {
            start: start.name.clone(),
            end: Some(end.name.clone()),
        }
        .to_a1_prefix()
    } else {
        format!("{}!", quote_sheet_name(&start.name))
    };
    Ok(format!("{prefix}{body}"))
}

fn filter_database_name_xml(sheet: &Sheet, local_sheet_id: usize) -> Result<String, CoreError> {
    let Some(filter) = &sheet.autofilter else {
        return Ok(String::new());
    };
    let start_col = col_to_letters(filter.range.start.col)
        .map_err(|source| error::xlsx_write(source.to_string()))?;
    let end_col = col_to_letters(filter.range.end.col)
        .map_err(|source| error::xlsx_write(source.to_string()))?;
    let referent = format!(
        "{}!${}${}:${}${}",
        quote_sheet_name(&sheet.name),
        start_col,
        filter.range.start.row + 1,
        end_col,
        filter.range.end.row + 1,
    );
    Ok(format!(
        r#"<definedName name="_xlnm._FilterDatabase" localSheetId="{local_sheet_id}" hidden="1">{}</definedName>"#,
        xml::escape(&referent),
    ))
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

fn sst_xml(
    sst: &IndexMap<StrId, u32>,
    intern: &Interners,
    count: u64,
) -> Result<Vec<u8>, CoreError> {
    let mut s = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><sst xmlns="{NS}" count="{}" uniqueCount="{}">"#,
        count,
        sst.len()
    );
    let mut items: Vec<(&StrId, &u32)> = sst.iter().collect();
    items.sort_by_key(|(_, i)| *i);
    for (id, _) in items {
        let text = intern
            .strings
            .get(*id)
            .ok_or_else(|| error::xlsx_write("shared string id disappeared"))?;
        s.push_str("<si>");
        if let Some(runs) = intern.strings.get_rich(*id) {
            s.push_str(&rich_text_xml(text, runs)?);
        } else {
            s.push_str(&t_elem(text));
        }
        s.push_str("</si>");
    }
    s.push_str("</sst>");
    Ok(s.into_bytes())
}

fn rich_text_xml(text: &str, runs: &[RichTextRun]) -> Result<String, CoreError> {
    let mut out = String::new();
    let mut cursor = 0usize;
    for run in runs {
        validate_font(&run.font)?;
        validate_color(run.font.color)?;
        let start = usize::try_from(run.start)
            .map_err(|_| error::xlsx_write("rich-text run offset overflow"))?;
        let len = usize::try_from(run.len)
            .map_err(|_| error::xlsx_write("rich-text run length overflow"))?;
        let end = start
            .checked_add(len)
            .ok_or_else(|| error::xlsx_write("rich-text run range overflow"))?;
        if start < cursor
            || end > text.len()
            || !text.is_char_boundary(start)
            || !text.is_char_boundary(end)
        {
            return Err(error::xlsx_write("invalid rich-text run range"));
        }
        if start > cursor {
            push_rich_run(&mut out, &Font::default(), &text[cursor..start]);
        }
        if end > start {
            push_rich_run(&mut out, &run.font, &text[start..end]);
        }
        cursor = end;
    }
    if cursor < text.len() {
        push_rich_run(&mut out, &Font::default(), &text[cursor..]);
    }
    Ok(out)
}

fn push_rich_run(out: &mut String, font: &Font, text: &str) {
    out.push_str("<r><rPr>");
    if font.bold {
        out.push_str("<b/>");
    }
    if font.italic {
        out.push_str("<i/>");
    }
    if font.strike {
        out.push_str("<strike/>");
    }
    match font.underline {
        Underline::None => {}
        Underline::Single => out.push_str("<u/>"),
        Underline::Double => out.push_str(r#"<u val="double"/>"#),
        Underline::SingleAccounting => out.push_str(r#"<u val="singleAccounting"/>"#),
        Underline::DoubleAccounting => out.push_str(r#"<u val="doubleAccounting"/>"#),
    }
    out.push_str(&format!(r#"<sz val="{}"/>"#, font.size_pt));
    out.push_str(&color_xml(&font.color));
    if !font.name.is_empty() {
        out.push_str(&format!(r#"<rFont val="{}"/>"#, xml::escape(&font.name)));
    }
    out.push_str("</rPr>");
    out.push_str(&t_elem(text));
    out.push_str("</r>");
}

fn t_elem(text: &str) -> String {
    if text.starts_with(' ') || text.ends_with(' ') || text.contains('\n') || text.contains('\t') {
        format!(
            r#"<t xml:space="preserve">{}</t>"#,
            xml::escape_ooxml_text(text)
        )
    } else {
        format!("<t>{}</t>", xml::escape_ooxml_text(text))
    }
}

fn styles_xml(
    wb: &Workbook,
    fonts: &[Font],
    fills: &[Fill],
    borders: &[omacell_core::style::Border],
    xfs: &[Style],
    dxfs: &[CfDxf],
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
    s.push_str("</cellXfs>");
    s.push_str(&super::data::dxfs_xml(dxfs));
    s.push_str("</styleSheet>");
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
            let attributes = match g.kind {
                GradientKind::Linear => format!(r#" degree="{}""#, g.degree),
                GradientKind::Path => format!(
                    r#" type="path" left="{}" right="{}" top="{}" bottom="{}""#,
                    g.left, g.right, g.top, g.bottom
                ),
            };
            let mut s = format!("<fill><gradientFill{attributes}>");
            for stop in &g.stops {
                let color = match stop.color {
                    Color::Auto => r#"<color auto="1"/>"#.into(),
                    _ => color_tag("color", &stop.color),
                };
                s.push_str(&format!(
                    r#"<stop position="{}">{}</stop>"#,
                    stop.position, color
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

fn validate_style(style: &Style) -> Result<(), CoreError> {
    validate_font(&style.font)?;
    validate_color(style.font.color)?;
    match &style.fill {
        Fill::None => {}
        Fill::Solid { fg } => validate_color(*fg)?,
        Fill::Pattern { fg, bg, .. } => {
            validate_color(*fg)?;
            validate_color(*bg)?;
        }
        Fill::Gradient(gradient) => {
            if !gradient.degree.is_finite()
                || ![gradient.left, gradient.right, gradient.top, gradient.bottom]
                    .iter()
                    .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
            {
                return Err(error::xlsx_write(
                    "gradient geometry is not finite or in range",
                ));
            }
            for stop in &gradient.stops {
                if !stop.position.is_finite() || !(0.0..=1.0).contains(&stop.position) {
                    return Err(error::xlsx_write("gradient stop is not finite or in range"));
                }
                validate_color(stop.color)?;
            }
        }
    }
    for side in [
        style.border.left,
        style.border.right,
        style.border.top,
        style.border.bottom,
    ] {
        validate_color(side.color)?;
    }
    Ok(())
}

fn validate_font(font: &Font) -> Result<(), CoreError> {
    if !font.size_pt.is_finite() || font.size_pt <= 0.0 {
        return Err(error::xlsx_write("font size is not finite and positive"));
    }
    Ok(())
}

fn validate_color(color: Color) -> Result<(), CoreError> {
    if let Color::Theme { tint, .. } = color
        && (!tint.is_finite() || !(-1.0..=1.0).contains(&tint))
    {
        return Err(error::xlsx_write("theme tint is not finite or in range"));
    }
    Ok(())
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn worksheet_xml(
    wb: &Workbook,
    sheet: &Sheet,
    extras: Option<&WorksheetExtras>,
    sst: &IndexMap<StrId, u32>,
    xf_index: &HashMap<Style, usize>,
    intern: &omacell_core::intern::Interners,
    sheet_ord: usize,
    package: Option<&OpcPackage>,
    persons: &BTreeMap<String, String>,
    dxfs: &[CfDxf],
    pivots: &[PivotTable],
    drawing_names: &mut drawing::PartNameAllocator,
    vml_shape_ids: &mut VmlShapeIdAllocator,
) -> Result<
    (
        Vec<u8>,
        Vec<(String, String, String, bool)>,
        Vec<(String, Vec<u8>, String)>,
    ),
    CoreError,
> {
    if !sheet.view.zoom.is_finite() || sheet.view.zoom <= 0.0 {
        return Err(error::xlsx_write("sheet zoom is not finite and positive"));
    }
    sheet.page_setup.validate()?;
    if sheet.view.freeze.rows >= MAX_ROWS
        || u32::from(sheet.view.freeze.cols) >= u32::from(MAX_COLS)
        || sheet.view.scroll_row >= MAX_ROWS
        || u32::from(sheet.view.scroll_col) >= u32::from(MAX_COLS)
        || sheet.view.selection.start.row >= MAX_ROWS
        || u32::from(sheet.view.selection.start.col) >= u32::from(MAX_COLS)
        || sheet.view.selection.end.row >= MAX_ROWS
        || u32::from(sheet.view.selection.end.col) >= u32::from(MAX_COLS)
    {
        return Err(error::xlsx_write("sheet view is outside the Excel grid"));
    }
    if let Some(color) = sheet.tab_color {
        validate_color(color)?;
    }
    let original_rels = match package {
        Some(package) => original_sheet_rels(package, &sheet.name, sheet_ord)?,
        None => Vec::new(),
    };
    let mut relationship_ids =
        RelationshipIdAllocator::new(&original_rels, "source worksheet", |_| true)?;
    let mut rels: Vec<(String, String, String, bool)> = Vec::new();
    let mut extra_parts = Vec::new();
    let uses_x14 = !sheet.sparklines.is_empty()
        || extras.is_some_and(|extra| worksheet_extras_contain(extra, b"x14:"));
    let uses_xr = extras.is_some_and(|extra| worksheet_extras_contain(extra, b"xr:"));
    let extension_namespaces = if uses_x14 {
        format!(r#" xmlns:x14="{NS_X14}" xmlns:xm="{NS_XM}""#)
    } else {
        String::new()
    };
    let revision_namespace = if uses_xr {
        format!(r#" xmlns:mc="{NS_MC}" xmlns:xr="{NS_XR}" mc:Ignorable="xr""#)
    } else {
        String::new()
    };
    let original_sheet_pr = original_sheet_element(package, &sheet.name, sheet_ord, "sheetPr")?;
    let original_sheet_protection =
        original_sheet_element(package, &sheet.name, sheet_ord, "sheetProtection")?;
    let mut s = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="{NS}" xmlns:r="{NS_R}"{extension_namespaces}{revision_namespace}>"#
    );
    let mut extensions = WorksheetExtensionFragments::default();
    let filter_mode = sheet
        .autofilter
        .as_ref()
        .is_some_and(|filter| !filter.columns.is_empty());
    s.push_str(&sheet_pr_xml(
        sheet,
        filter_mode,
        original_sheet_pr.as_ref(),
    )?);
    s.push_str(&sheet_views_xml(sheet));
    s.push_str(&cols_xml(sheet));
    s.push_str("<sheetData>");
    let mut cells_by_row: IndexMap<u32, Vec<(u16, CellSlot)>> = IndexMap::new();
    for (row, col, slot) in sheet.store.iter() {
        cells_by_row.entry(row).or_default().push((col, slot));
    }
    let hidden_rows: Vec<u32> = sheet.geometry.rows.iter_hidden().collect();
    let custom_rows: Vec<(u32, u32)> = sheet.geometry.rows.iter_custom().collect();
    let outline_rows: Vec<(u32, u8)> = sheet.geometry.rows.iter_outline().collect();
    let collapsed_rows: Vec<u32> = sheet.geometry.rows.iter_collapsed().collect();
    let mut row_idxs: Vec<u32> = cells_by_row.keys().copied().collect();
    row_idxs.extend_from_slice(&hidden_rows);
    row_idxs.extend(custom_rows.iter().map(|(i, _)| *i));
    row_idxs.extend(outline_rows.iter().map(|(i, _)| *i));
    row_idxs.extend_from_slice(&collapsed_rows);
    row_idxs.sort_unstable();
    row_idxs.dedup();
    for row in row_idxs {
        let r1 = row + 1;
        let mut attrs = format!(r#" r="{r1}""#);
        if hidden_rows.contains(&row) {
            attrs.push_str(r#" hidden="1""#);
        }
        let outline = sheet.geometry.rows.outline_level(row);
        if outline > 0 {
            attrs.push_str(&format!(r#" outlineLevel="{outline}""#));
        }
        if sheet.geometry.rows.is_collapsed(row) {
            attrs.push_str(r#" collapsed="1""#);
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
            let array_formula = sheet
                .array_formula_at(row, *col)
                .filter(|formula| formula.anchor.row == row && formula.anchor.col == *col);
            s.push_str(&cell_xml(
                row,
                *col,
                slot,
                array_formula,
                sst,
                xf_index,
                intern,
            )?);
        }
        s.push_str("</row>");
    }
    s.push_str("</sheetData>");
    if sheet.protection.enabled {
        if let Some(raw) = original_sheet_protection
            .as_ref()
            .filter(|element| sheet_protection_matches(&element.attrs, &sheet.protection))
        {
            s.push_str(
                std::str::from_utf8(&raw.raw)
                    .map_err(|_| error::xlsx_write("preserved sheet protection is not UTF-8"))?,
            );
        } else {
            let password = sheet
                .protection
                .password
                .as_deref()
                .map(std::str::from_utf8)
                .transpose()
                .map_err(|_| error::xlsx_write("sheet protection verifier is not UTF-8"))?
                .map(|value| format!(r#" password="{}""#, xml::escape(value)))
                .unwrap_or_default();
            let allow = &sheet.protection.allow;
            s.push_str(&format!(
                r#"<sheetProtection sheet="1"{password} selectLockedCells="{}" selectUnlockedCells="{}" formatCells="{}" insertRows="{}" insertColumns="{}" sort="{}" autoFilter="{}"/>"#,
                u8::from(!allow.select_locked),
                u8::from(!allow.select_unlocked),
                u8::from(!allow.format_cells),
                u8::from(!allow.insert_rows),
                u8::from(!allow.insert_cols),
                u8::from(!allow.sort),
                u8::from(!allow.auto_filter),
            ));
        }
    }
    if !sheet.protection.protected_ranges.is_empty() {
        s.push_str("<protectedRanges>");
        for range in &sheet.protection.protected_ranges {
            let sqref = range
                .ranges
                .iter()
                .map(|range| range.to_a1())
                .collect::<Vec<_>>()
                .join(" ");
            let password = range
                .password
                .as_deref()
                .map(std::str::from_utf8)
                .transpose()
                .map_err(|_| error::xlsx_write("protected-range verifier is not UTF-8"))?
                .map(|value| format!(r#" password="{}""#, xml::escape(value)))
                .unwrap_or_default();
            s.push_str(&format!(
                r#"<protectedRange name="{}" sqref="{}"{password}/>"#,
                xml::escape(&range.name),
                xml::escape(&sqref),
            ));
        }
        s.push_str("</protectedRanges>");
    }
    if let Some(raw) = extras
        .map(|extra| extra.autofilter_xml.as_slice())
        .filter(|raw| {
            !raw.is_empty()
                && super::data::autofilter_extras_match(raw, dxfs, sheet.autofilter.as_ref())
        })
    {
        push_fragment(&mut s, raw, "automatic filter", &["autoFilter"])?;
    } else if let Some(filter) = &sheet.autofilter {
        if let Some(raw_ref) = extras.and_then(|extra| extra.autofilter.as_deref())
            && filter.columns.is_empty()
            && raw_ref == filter.range.to_a1()
        {
            s.push_str(&format!(r#"<autoFilter ref="{}"/>"#, xml::escape(raw_ref)));
        } else {
            s.push_str(&super::data::modeled_autofilter(filter, dxfs));
        }
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
    if let Some(ex) = extras.filter(|ex| {
        !ex.conditional_formatting_xml.is_empty()
            && super::data::cond_format_extras_match(
                &ex.conditional_formatting_xml,
                dxfs,
                &sheet.cond_formats,
            )
    }) {
        for blob in &ex.conditional_formatting_xml {
            if is_x14_fragment(blob) {
                extensions.conditional_formatting.push(validated_fragment(
                    blob,
                    "conditional formatting",
                    &["conditionalFormatting"],
                )?);
            } else {
                push_fragment(
                    &mut s,
                    blob,
                    "conditional formatting",
                    &["conditionalFormatting"],
                )?;
            }
        }
    } else {
        for xml in super::data::modeled_cond_formats(&sheet.cond_formats, dxfs) {
            push_fragment(
                &mut s,
                xml.as_bytes(),
                "conditional formatting",
                &["conditionalFormatting"],
            )?;
        }
    }
    if let Some(ex) = extras.filter(|ex| {
        !ex.data_validations_xml.is_empty()
            && super::data::validation_extras_match(&ex.data_validations_xml, &sheet.validations)
    }) {
        for blob in &ex.data_validations_xml {
            if is_x14_fragment(blob) {
                set_single_extension(
                    &mut extensions.data_validations,
                    validated_fragment(blob, "data validation", &["dataValidations"])?,
                    "data validation",
                )?;
            } else {
                push_fragment(&mut s, blob, "data validation", &["dataValidations"])?;
            }
        }
    } else if let Some(xml) = super::data::modeled_validations(&sheet.validations) {
        push_fragment(
            &mut s,
            xml.as_bytes(),
            "data validation",
            &["dataValidations"],
        )?;
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
            let display = link
                .display
                .as_ref()
                .map(|d| format!(r#" display="{}""#, xml::escape(d)))
                .unwrap_or_default();
            let tooltip = link
                .tooltip
                .as_ref()
                .map(|value| format!(r#" tooltip="{}""#, xml::escape(value)))
                .unwrap_or_default();
            if is_internal_hyperlink(&link.target) {
                s.push_str(&format!(
                    r#"<hyperlink ref="{addr}" location="{}"{tooltip}{display}/>"#,
                    xml::escape(&link.target)
                ));
            } else {
                let id = relationship_ids.next()?;
                rels.push((id.clone(), REL_HYPER.into(), link.target.clone(), true));
                s.push_str(&format!(
                    r#"<hyperlink ref="{addr}" r:id="{id}"{tooltip}{display}/>"#
                ));
            }
        }
        s.push_str("</hyperlinks>");
    }
    let print_roots = [
        "pageSetup",
        "pageMargins",
        "printOptions",
        "headerFooter",
        "rowBreaks",
        "colBreaks",
    ];
    if let Some(ex) = extras.filter(|ex| xlsx_print::extras_match(&ex.print_xml, &sheet.page_setup))
    {
        for blob in &ex.print_xml {
            push_fragment(&mut s, blob, "print settings", &print_roots)?;
        }
    } else if !sheet.page_setup.is_default() {
        for xml in xlsx_print::modeled_print_xml(&sheet.page_setup) {
            push_fragment(&mut s, xml.as_bytes(), "print settings", &print_roots)?;
        }
    }
    if sheet
        .notes
        .keys()
        .any(|coordinate| sheet.comments.contains_key(coordinate))
    {
        return Err(error::xlsx_write(
            "a cell cannot contain both a note and a threaded comment",
        ));
    }
    let threaded_comments = if sheet.comments.is_empty() {
        None
    } else {
        Some(threaded_comments_xml(sheet, sheet_ord, persons)?)
    };
    let annotation_cells: BTreeSet<(u32, u16)> = sheet
        .notes
        .keys()
        .chain(sheet.comments.keys())
        .copied()
        .collect();
    let mut drawing_xml = String::new();
    let mut vml_xml = String::new();
    let worksheet_name = format!("xl/worksheets/sheet{}.xml", sheet_ord + 1);
    let source_drawing = original_rels
        .iter()
        .find(|relationship| relationship.rel_type == REL_DRAWING && !relationship.external);
    let modeled = drawing::chart_parts(
        wb,
        sheet,
        sheet_ord,
        package,
        source_drawing.map(|relationship| relationship.target.as_str()),
        drawing_names,
    )?;
    for relationship in &original_rels {
        if relationship.rel_type == REL_HYPER
            || relationship.rel_type == REL_TABLE
            || relationship.rel_type == REL_PIVOT_TABLE
            || relationship.rel_type == REL_COMMENTS
            || relationship.rel_type == REL_THREADED_COMMENTS
            || (relationship.rel_type == REL_DRAWING && modeled.is_some())
        {
            continue;
        }
        let target = if relationship.external {
            relationship.target.clone()
        } else {
            relative_target(&worksheet_name, &relationship.target)
        };
        if relationship.rel_type == REL_DRAWING {
            drawing_xml = format!(r#"<drawing r:id="{}"/>"#, relationship.id);
        } else if relationship.rel_type == REL_VML {
            vml_xml = format!(r#"<legacyDrawing r:id="{}"/>"#, relationship.id);
        }
        rels.push((
            relationship.id.clone(),
            relationship.rel_type.clone(),
            target,
            relationship.external,
        ));
    }
    if let Some(parts) = modeled {
        let id = source_drawing
            .map(|relationship| relationship.id.clone())
            .map_or_else(|| relationship_ids.next(), Ok)?;
        drawing_xml = format!(r#"<drawing r:id="{id}"/>"#);
        rels.push((
            id,
            REL_DRAWING.into(),
            relative_target(&worksheet_name, &parts.drawing_name),
            false,
        ));
        extra_parts.extend(parts.parts);
    }
    if let Some(source_vml) = original_rels
        .iter()
        .rev()
        .find(|relationship| relationship.rel_type == REL_VML && !relationship.external)
        && let Some(source_part) = package.and_then(|package| package.part(&source_vml.target))
    {
        let reconciled =
            reconcile_comments_vml(&source_part.bytes, &annotation_cells, vml_shape_ids)?;
        if reconciled != source_part.bytes {
            extra_parts.push((source_vml.target.clone(), reconciled, CT_VML.into()));
        }
    }
    if !annotation_cells.is_empty() && vml_xml.is_empty() {
        let id = relationship_ids.next()?;
        let number = sheet_ord + 1;
        let name = drawing_names.take(format!("xl/drawings/vmlDrawing{number}.vml"))?;
        rels.push((
            id.clone(),
            REL_VML.into(),
            relative_target(&worksheet_name, &name),
            false,
        ));
        extra_parts.push((
            name,
            comments_vml_xml(&annotation_cells, vml_shape_ids)?,
            CT_VML.into(),
        ));
        vml_xml = format!(r#"<legacyDrawing r:id="{id}"/>"#);
    }
    s.push_str(&drawing_xml);
    s.push_str(&vml_xml);
    let tables: Vec<&Table> = wb.tables().iter().filter(|t| t.sheet == sheet.id).collect();
    if !tables.is_empty() {
        s.push_str(&format!(r#"<tableParts count="{}">"#, tables.len()));
        for table in tables {
            validate_table(table)?;
            let id = relationship_ids.next()?;
            let table_number = table.id.index().saturating_add(1);
            let tname = format!("xl/tables/table{table_number}.xml");
            rels.push((
                id.clone(),
                REL_TABLE.into(),
                format!("../tables/table{table_number}.xml"),
                false,
            ));
            extra_parts.push((tname, table_xml(table, table_number), CT_TBL.into()));
            s.push_str(&format!(r#"<tablePart r:id="{id}"/>"#));
        }
        s.push_str("</tableParts>");
    }
    for pivot in pivots.iter().filter(|pivot| pivot.dest_sheet == sheet.id) {
        let extra = if let Some(pkg) = package {
            super::pivot::preserved_table_parts(pkg, pivot)
        } else {
            None
        };
        let extra = match extra {
            Some(parts) => parts,
            None => super::pivot::table_parts(wb, pivot)?,
        };
        let id = relationship_ids.next()?;
        rels.push((id, REL_PIVOT_TABLE.into(), extra.rel_target, false));
        extra_parts.extend(extra.parts);
    }
    if let Some(ex) = extras.filter(|ex| {
        !ex.sparkline_xml.is_empty()
            && drawing::sparkline_extras_match(&ex.sparkline_xml, wb, sheet)
    }) {
        for blob in &ex.sparkline_xml {
            set_single_extension(
                &mut extensions.sparklines,
                validated_fragment(blob, "sparkline", &["sparklineGroups"])?,
                "sparkline",
            )?;
        }
    } else if let Some(blob) = drawing::sparkline_xml(wb, sheet) {
        set_single_extension(
            &mut extensions.sparklines,
            validated_fragment(&blob, "sparkline", &["sparklineGroups"])?,
            "sparkline",
        )?;
    }
    if !annotation_cells.is_empty() {
        let id = relationship_ids.next()?;
        let cname = format!("xl/comments{}.xml", sheet_ord + 1);
        rels.push((
            id,
            REL_COMMENTS.into(),
            format!("../comments{}.xml", sheet_ord + 1),
            false,
        ));
        let placeholders = threaded_comments
            .as_ref()
            .map(|comments| comments.placeholders.as_slice())
            .unwrap_or(&[]);
        extra_parts.push((cname, comments_xml(sheet, placeholders)?, CT_CMT.into()));
    }
    if let Some(threaded_comments) = threaded_comments {
        let id = relationship_ids.next()?;
        let number = sheet_ord + 1;
        let name = format!("xl/threadedComments/threadedComment{number}.xml");
        rels.push((
            id,
            REL_THREADED_COMMENTS.into(),
            format!("../threadedComments/threadedComment{number}.xml"),
            false,
        ));
        extra_parts.push((name, threaded_comments.bytes, CT_THREADED_CMT.into()));
    }
    push_worksheet_extensions(&mut s, &extensions);
    s.push_str("</worksheet>");
    Ok((s.into_bytes(), rels, extra_parts))
}

#[derive(Default)]
struct WorksheetExtensionFragments {
    conditional_formatting: Vec<String>,
    data_validations: Option<String>,
    sparklines: Option<String>,
}

fn worksheet_extras_contain(extras: &WorksheetExtras, needle: &[u8]) -> bool {
    std::iter::once(extras.autofilter_xml.as_slice())
        .chain(extras.print_xml.iter().map(Vec::as_slice))
        .chain(extras.conditional_formatting_xml.iter().map(Vec::as_slice))
        .chain(extras.data_validations_xml.iter().map(Vec::as_slice))
        .chain(extras.sparkline_xml.iter().map(Vec::as_slice))
        .any(|fragment| {
            fragment
                .windows(needle.len())
                .any(|window| window == needle)
        })
}

fn is_x14_fragment(fragment: &[u8]) -> bool {
    fragment.starts_with(b"<x14:")
}

fn validated_fragment(
    bytes: &[u8],
    kind: &str,
    allowed_roots: &[&str],
) -> Result<String, CoreError> {
    let mut fragment = String::new();
    push_fragment(&mut fragment, bytes, kind, allowed_roots)?;
    Ok(fragment)
}

fn set_single_extension(
    slot: &mut Option<String>,
    fragment: String,
    kind: &str,
) -> Result<(), CoreError> {
    if slot.replace(fragment).is_some() {
        return Err(error::xlsx_write(format!(
            "worksheet has more than one {kind} extension payload"
        )));
    }
    Ok(())
}

fn push_worksheet_extensions(out: &mut String, extensions: &WorksheetExtensionFragments) {
    if extensions.conditional_formatting.is_empty()
        && extensions.data_validations.is_none()
        && extensions.sparklines.is_none()
    {
        return;
    }
    out.push_str("<extLst>");
    if !extensions.conditional_formatting.is_empty() {
        out.push_str(&format!(
            r#"<ext uri="{EXT_CONDITIONAL_FORMATTING}"><x14:conditionalFormattings>"#
        ));
        for fragment in &extensions.conditional_formatting {
            out.push_str(fragment);
        }
        out.push_str("</x14:conditionalFormattings></ext>");
    }
    if let Some(fragment) = &extensions.data_validations {
        out.push_str(&format!(r#"<ext uri="{EXT_DATA_VALIDATIONS}">"#));
        out.push_str(fragment);
        out.push_str("</ext>");
    }
    if let Some(fragment) = &extensions.sparklines {
        out.push_str(&format!(r#"<ext uri="{EXT_SPARKLINES}">"#));
        out.push_str(fragment);
        out.push_str("</ext>");
    }
    out.push_str("</extLst>");
}

fn push_fragment(
    out: &mut String,
    bytes: &[u8],
    kind: &str,
    allowed_roots: &[&str],
) -> Result<(), CoreError> {
    let fragment = std::str::from_utf8(bytes)
        .map_err(|_| error::xlsx_write(format!("{kind} XML fragment is not UTF-8")))?;
    if fragment.starts_with('\u{feff}') || fragment.contains("<?xml") {
        return Err(error::xlsx_write(format!(
            "{kind} XML fragment contains a document declaration"
        )));
    }
    let mut reader = xml::XmlReader::new(bytes);
    let mut depth = 0u32;
    let mut roots = 0u32;
    while let Some(event) = reader.next()? {
        match event {
            xml::XmlEvent::Start { name, .. } => {
                if depth == 0 {
                    roots += 1;
                    if !allowed_roots.contains(&name.as_str()) {
                        return Err(error::xlsx_write(format!(
                            "{kind} XML has unexpected root {name:?}"
                        )));
                    }
                }
                depth += 1;
            }
            xml::XmlEvent::Empty { name, .. } => {
                if depth == 0 {
                    roots += 1;
                    if !allowed_roots.contains(&name.as_str()) {
                        return Err(error::xlsx_write(format!(
                            "{kind} XML has unexpected root {name:?}"
                        )));
                    }
                }
            }
            xml::XmlEvent::End { .. } => depth = depth.saturating_sub(1),
            xml::XmlEvent::Text(text) if depth == 0 && !text.trim().is_empty() => {
                return Err(error::xlsx_write(format!(
                    "{kind} XML has text outside its root"
                )));
            }
            xml::XmlEvent::Text(_) => {}
        }
    }
    if roots != 1 || depth != 0 {
        return Err(error::xlsx_write(format!(
            "{kind} XML must contain exactly one complete root"
        )));
    }
    out.push_str(fragment);
    Ok(())
}

fn original_sheet_rels(
    pkg: &OpcPackage,
    sheet_name: &str,
    sheet_ord: usize,
) -> Result<Vec<super::opc::Relationship>, CoreError> {
    let Some(part_name) = original_sheet_part_name(pkg, sheet_name, sheet_ord)? else {
        return Ok(Vec::new());
    };
    pkg.rels_for(&part_name)
}

fn extras_for_sheet<'a>(
    extras: &'a HashMap<String, WorksheetExtras>,
    package: Option<&OpcPackage>,
    current_name: &str,
    sheet_ord: usize,
) -> Result<Option<&'a WorksheetExtras>, CoreError> {
    if let Some(package) = package
        && let Some(source_name) = original_sheet_name(package, sheet_ord)?
        && let Some(source_extras) = extras.get(&source_name)
    {
        return Ok(Some(source_extras));
    }
    Ok(extras.get(current_name))
}

fn original_sheet_name(pkg: &OpcPackage, sheet_ord: usize) -> Result<Option<String>, CoreError> {
    let Ok(workbook) = pkg.workbook_part() else {
        return Ok(None);
    };
    let mut reader = xml::XmlReader::new(&workbook.bytes);
    let mut in_sheets = false;
    let mut index = 0usize;
    while let Some(event) = reader.next()? {
        match event {
            xml::XmlEvent::Start { name, .. } if name == "sheets" => in_sheets = true,
            xml::XmlEvent::End { name } if name == "sheets" => in_sheets = false,
            xml::XmlEvent::Start { name, attrs } | xml::XmlEvent::Empty { name, attrs }
                if in_sheets && name == "sheet" =>
            {
                if index == sheet_ord {
                    return Ok(xml::attr(&attrs, "name").map(ToOwned::to_owned));
                }
                index = index.saturating_add(1);
            }
            _ => {}
        }
    }
    Ok(None)
}

fn original_sheet_part_name(
    pkg: &OpcPackage,
    sheet_name: &str,
    sheet_ord: usize,
) -> Result<Option<String>, CoreError> {
    let Ok(workbook) = pkg.workbook_part() else {
        return Ok(None);
    };
    let Ok(rels) = pkg.rels_for(&workbook.name) else {
        return Ok(None);
    };
    let mut reader = xml::XmlReader::new(&workbook.bytes);
    let mut in_sheets = false;
    let mut sheet_rids = Vec::new();
    let mut matching_rid = None;
    while let Some(event) = reader.next()? {
        match event {
            xml::XmlEvent::Start { name, .. } if name == "sheets" => in_sheets = true,
            xml::XmlEvent::End { name } if name == "sheets" => in_sheets = false,
            xml::XmlEvent::Start { name, attrs } | xml::XmlEvent::Empty { name, attrs }
                if in_sheets && name == "sheet" =>
            {
                let rid = xml::attr(&attrs, "id").unwrap_or("").to_string();
                if xml::attr(&attrs, "name").is_some_and(|name| name == sheet_name) {
                    matching_rid = Some(rid.clone());
                }
                sheet_rids.push(rid);
            }
            _ => {}
        }
    }
    let rid = matching_rid.or_else(|| sheet_rids.get(sheet_ord).cloned());
    let Some(rel) = rid.and_then(|rid| {
        rels.iter()
            .find(|rel| rel.id == rid && rel.rel_type == REL_WS)
    }) else {
        return Ok(None);
    };
    Ok(Some(rel.target.clone()))
}

fn original_sheet_element(
    package: Option<&OpcPackage>,
    sheet_name: &str,
    sheet_ord: usize,
    wanted: &str,
) -> Result<Option<PreservedXmlElement>, CoreError> {
    let Some(package) = package else {
        return Ok(None);
    };
    let Some(part_name) = original_sheet_part_name(package, sheet_name, sheet_ord)? else {
        return Ok(None);
    };
    let Some(part) = package.part(&part_name) else {
        return Ok(None);
    };
    first_xml_element(&part.bytes, wanted)
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
    let top_left = if v.scroll_row > 0 || v.scroll_col > 0 {
        let col = col_to_letters(v.scroll_col).unwrap_or_else(|_| "A".into());
        format!(r#" topLeftCell="{col}{}""#, v.scroll_row + 1)
    } else {
        String::new()
    };
    let mut pane = String::new();
    if v.freeze.rows > 0 || v.freeze.cols > 0 {
        let top_left = format!(
            "{}{}",
            col_to_letters(v.freeze.cols).unwrap_or_else(|_| "A".into()),
            v.freeze.rows + 1
        );
        let active_pane = match (v.freeze.rows > 0, v.freeze.cols > 0) {
            (true, true) => "bottomRight",
            (true, false) => "bottomLeft",
            (false, true) => "topRight",
            (false, false) => "topLeft",
        };
        pane = format!(
            r#"<pane xSplit="{}" ySplit="{}" topLeftCell="{top_left}" activePane="{active_pane}" state="frozen"/>"#,
            v.freeze.cols, v.freeze.rows
        );
    } else if let Some(split) = v.split {
        pane = format!(
            r#"<pane xSplit="{}" ySplit="{}" state="split"/>"#,
            split_pixels_to_twips(split.x_px),
            split_pixels_to_twips(split.y_px)
        );
    }
    let selection = v.selection.to_a1();
    let active_cell = v.selection.start.to_a1();
    format!(
        r#"<sheetViews><sheetView workbookViewId="0"{zoom}{grid}{formulas}{top_left}>{pane}<selection activeCell="{}" sqref="{}"/></sheetView></sheetViews>"#,
        xml::escape(&active_cell),
        xml::escape(&selection)
    )
}

fn is_internal_hyperlink(target: &str) -> bool {
    target.starts_with('#')
        || (!target.contains("://")
            && !target.starts_with("mailto:")
            && !target.starts_with("file:")
            && target.contains('!'))
}

fn cols_xml(sheet: &Sheet) -> String {
    let hidden: Vec<u32> = sheet.geometry.cols.iter_hidden().collect();
    let custom: Vec<(u32, u32)> = sheet.geometry.cols.iter_custom().collect();
    let outline: Vec<(u32, u8)> = sheet.geometry.cols.iter_outline().collect();
    let collapsed: Vec<u32> = sheet.geometry.cols.iter_collapsed().collect();
    if hidden.is_empty() && custom.is_empty() && outline.is_empty() && collapsed.is_empty() {
        return String::new();
    }
    let mut idxs: Vec<u32> = hidden
        .iter()
        .copied()
        .chain(custom.iter().map(|(i, _)| *i))
        .chain(outline.iter().map(|(i, _)| *i))
        .chain(collapsed.iter().copied())
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
        let outline_attr = match sheet.geometry.cols.outline_level(i) {
            0 => String::new(),
            level => format!(r#" outlineLevel="{level}""#),
        };
        let collapsed_attr = if collapsed.contains(&i) {
            r#" collapsed="1""#
        } else {
            ""
        };
        s.push_str(&format!(
            r#"<col min="{min}" max="{min}" width="{width}" customWidth="1"{hidden_attr}{outline_attr}{collapsed_attr}/>"#
        ));
    }
    s.push_str("</cols>");
    s
}

fn cell_xml(
    row: u32,
    col: u16,
    slot: &CellSlot,
    array_formula: Option<&ArrayFormula>,
    sst: &IndexMap<StrId, u32>,
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
    let formula = slot
        .formula
        .and_then(|id| intern.formulas.get(id))
        .filter(|source| !super::ai_formula::is_ai_formula(source));
    if let Some(src) = formula {
        let xlsx_formula = super::formula::to_xlsx(src);
        let body = xlsx_formula.strip_prefix('=').unwrap_or(&xlsx_formula);
        if let Some(array_formula) = array_formula {
            inner.push_str(&format!(
                r#"<f t="array" ref="{}">{}</f>"#,
                array_formula.range.to_a1(),
                xml::escape(body)
            ));
        } else {
            inner.push_str(&format!("<f>{}</f>", xml::escape(body)));
        }
    }
    match slot.value {
        Value::Number(n) => {
            if !n.is_finite() {
                return Err(error::xlsx_write(format!(
                    "cell {addr} contains a non-finite number"
                )));
            }
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
                if formula.is_some() {
                    attrs.push_str(r#" t="str""#);
                    inner.push_str(&format!("<v>{}</v>", xml::escape_ooxml_text(text)));
                } else if let Some(idx) = sst.get(&id) {
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

fn table_xml(table: &Table, table_number: u32) -> Vec<u8> {
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
    let autofilter = if table.has_header {
        format!(r#"<autoFilter ref="{start}:{end}"/>"#)
    } else {
        String::new()
    };
    let mut cols = String::new();
    for (i, c) in table.columns.iter().enumerate() {
        let totals = c
            .totals_fn
            .as_deref()
            .map(|fn_name| format!(r#" totalsRowFunction="{fn_name}""#))
            .unwrap_or_default();
        cols.push_str(&format!(
            r#"<tableColumn id="{}" name="{}"{totals}/>"#,
            i + 1,
            xml::escape(&c.name)
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><table xmlns="{NS}" id="{table_number}" name="{}" displayName="{}" ref="{start}:{end}" headerRowCount="{header}" totalsRowCount="{totals}">{autofilter}<tableColumns count="{}">{cols}</tableColumns><tableStyleInfo name="{}" showFirstColumn="0" showLastColumn="0" showRowStripes="{}" showColumnStripes="{}"/></table>"#,
        xml::escape(&table.name),
        xml::escape(&table.name),
        table.columns.len(),
        xml::escape(&table.style_name),
        u8::from(table.banded_rows),
        u8::from(table.banded_cols)
    )
    .into_bytes()
}

fn validate_table(table: &Table) -> Result<(), CoreError> {
    let width = u32::from(table.end_col)
        .checked_sub(u32::from(table.start_col))
        .and_then(|width| width.checked_add(1));
    if table.start_row > table.end_row
        || table.end_row >= MAX_ROWS
        || u32::from(table.end_col) >= u32::from(MAX_COLS)
        || width != u32::try_from(table.columns.len()).ok()
    {
        return Err(error::xlsx_write(format!(
            "table {:?} has an invalid range or column count",
            table.name
        )));
    }
    Ok(())
}

fn threaded_persons(sheets: &[&Sheet]) -> BTreeMap<String, String> {
    fn collect(
        comment: &omacell_core::sheet::Comment,
        authors: &mut std::collections::BTreeSet<String>,
    ) {
        authors.insert(comment.author.clone());
        for reply in &comment.replies {
            collect(reply, authors);
        }
    }

    let mut authors = std::collections::BTreeSet::new();
    for sheet in sheets {
        for comment in sheet.comments.values() {
            collect(comment, &mut authors);
        }
    }
    authors
        .into_iter()
        .enumerate()
        .map(|(index, author)| (author, deterministic_guid(1, index as u64 + 1)))
        .collect()
}

fn deterministic_guid(namespace: u32, ordinal: u64) -> String {
    format!("{{{namespace:08X}-0000-4000-8000-{ordinal:012X}}}")
}

fn persons_xml(persons: &BTreeMap<String, String>) -> Vec<u8> {
    let mut xml_out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><personList xmlns="http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments">"#,
    );
    for (author, id) in persons {
        xml_out.push_str(&format!(
            r#"<person displayName="{}" id="{}" userId="{}" providerId="None"/>"#,
            xml::escape(author),
            xml::escape(id),
            xml::escape(author),
        ));
    }
    xml_out.push_str("</personList>");
    xml_out.into_bytes()
}

struct LegacyThreadPlaceholder {
    cell_ref: String,
    id: String,
}

struct ThreadedCommentsPart {
    bytes: Vec<u8>,
    placeholders: Vec<LegacyThreadPlaceholder>,
}

fn threaded_comments_xml(
    sheet: &Sheet,
    sheet_ord: usize,
    persons: &BTreeMap<String, String>,
) -> Result<ThreadedCommentsPart, CoreError> {
    let mut comments: Vec<_> = sheet.comments.iter().collect();
    comments.sort_by_key(|((row, col), _)| (*row, *col));
    let mut xml_out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><ThreadedComments xmlns="http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments">"#,
    );
    let mut ordinal = 1u64;
    let mut placeholders = Vec::with_capacity(comments.len());
    for ((row, col), comment) in comments {
        let cell_ref = format!(
            "{}{}",
            col_to_letters(*col).map_err(|error| error::xlsx_write(error.to_string()))?,
            row + 1
        );
        append_thread_comment_xml(
            &mut xml_out,
            comment,
            &cell_ref,
            None,
            sheet_ord,
            persons,
            &mut ordinal,
            &mut placeholders,
            0,
        )?;
    }
    xml_out.push_str("</ThreadedComments>");
    Ok(ThreadedCommentsPart {
        bytes: xml_out.into_bytes(),
        placeholders,
    })
}

#[allow(clippy::too_many_arguments)]
fn append_thread_comment_xml(
    out: &mut String,
    comment: &omacell_core::sheet::Comment,
    cell_ref: &str,
    parent_id: Option<&str>,
    sheet_ord: usize,
    persons: &BTreeMap<String, String>,
    ordinal: &mut u64,
    placeholders: &mut Vec<LegacyThreadPlaceholder>,
    depth: usize,
) -> Result<(), CoreError> {
    if depth >= 64 {
        return Err(error::xlsx_write(
            "threaded comment nesting exceeds 64 levels",
        ));
    }
    let person_id = persons
        .get(&comment.author)
        .ok_or_else(|| error::xlsx_write("threaded comment author has no person record"))?;
    let id = deterministic_guid(u32::try_from(sheet_ord + 2).unwrap_or(u32::MAX), *ordinal);
    *ordinal = ordinal.saturating_add(1);
    if parent_id.is_none() {
        placeholders.push(LegacyThreadPlaceholder {
            cell_ref: cell_ref.to_string(),
            id: id.clone(),
        });
    }
    let parent = parent_id
        .map(|value| format!(r#" parentId="{}""#, xml::escape(value)))
        .unwrap_or_default();
    let done = if comment.resolved { r#" done="1""# } else { "" };
    out.push_str(&format!(
        r#"<threadedComment ref="{}" dT="1970-01-01T00:00:00Z" personId="{}" id="{}"{parent}{done}><text>{}</text></threadedComment>"#,
        xml::escape(cell_ref),
        xml::escape(person_id),
        xml::escape(&id),
        xml::escape(&comment.text),
    ));
    for reply in &comment.replies {
        append_thread_comment_xml(
            out,
            reply,
            cell_ref,
            Some(&id),
            sheet_ord,
            persons,
            ordinal,
            placeholders,
            depth + 1,
        )?;
    }
    Ok(())
}

fn comments_xml(
    sheet: &Sheet,
    placeholders: &[LegacyThreadPlaceholder],
) -> Result<Vec<u8>, CoreError> {
    let mut authors: Vec<String> = Vec::new();
    let mut notes: Vec<_> = sheet.notes.iter().collect();
    notes.sort_by_key(|((r, c), _)| (*r, *c));
    for (_, n) in &notes {
        let a = n.author.clone().unwrap_or_default();
        if !authors.iter().any(|x| x == &a) {
            authors.push(a);
        }
    }
    for placeholder in placeholders {
        authors.push(format!("tc={}", placeholder.id));
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
            col_to_letters(*col).map_err(|error| error::xlsx_write(error.to_string()))?,
            row + 1
        );
        let author = n.author.as_deref().unwrap_or("");
        let aid = authors
            .iter()
            .position(|candidate| candidate == author)
            .unwrap_or(0);
        s.push_str(&format!(
            r#"<comment ref="{addr}" authorId="{aid}"><text>{}</text></comment>"#,
            t_elem(&n.text)
        ));
    }
    for placeholder in placeholders {
        let author = format!("tc={}", placeholder.id);
        let author_id = authors
            .iter()
            .position(|candidate| candidate == &author)
            .ok_or_else(|| error::xlsx_write("threaded comment placeholder has no author"))?;
        s.push_str(&format!(
            r#"<comment ref="{}" authorId="{author_id}"><text>{}</text></comment>"#,
            xml::escape(&placeholder.cell_ref),
            t_elem("")
        ));
    }
    s.push_str("</commentList></comments>");
    Ok(s.into_bytes())
}

fn comments_vml_xml(
    cells: &BTreeSet<(u32, u16)>,
    shape_ids: &mut VmlShapeIdAllocator,
) -> Result<Vec<u8>, CoreError> {
    let mut s = String::from(
        r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:x="urn:schemas-microsoft-com:office:excel"><o:shapelayout v:ext="edit"><o:idmap v:ext="edit" data="1"/></o:shapelayout><v:shapetype id="_x0000_t202" coordsize="21600,21600" o:spt="202" path="m,l,21600r21600,l21600,xe"><v:stroke joinstyle="miter"/><v:path gradientshapeok="t" o:connecttype="rect"/></v:shapetype>"#,
    );
    s.push_str(&comments_vml_shapes(cells, shape_ids)?);
    s.push_str("</xml>");
    Ok(s.into_bytes())
}

fn comments_vml_shapes(
    cells: &BTreeSet<(u32, u16)>,
    shape_ids: &mut VmlShapeIdAllocator,
) -> Result<String, CoreError> {
    let mut s = String::new();
    for (index, &(row, col)) in cells.iter().enumerate() {
        let end_col = u32::from(col).saturating_add(2).min(16_383);
        let end_row = row.saturating_add(4).min(1_048_575);
        let shape_id = shape_ids.next()?;
        s.push_str(&format!(
            r##"<v:shape id="{shape_id}" type="#_x0000_t202" style="position:absolute;margin-left:80pt;margin-top:5pt;width:108pt;height:59.25pt;z-index:{};visibility:hidden" fillcolor="#ffffe1" o:insetmode="auto"><v:fill color2="#ffffe1"/><v:shadow on="t" color="black" obscured="t"/><v:path o:connecttype="none"/><v:textbox style="mso-direction-alt:auto"><div style="text-align:left"/></v:textbox><x:ClientData ObjectType="Note"><x:MoveWithCells/><x:SizeWithCells/><x:Anchor>{col}, 15, {row}, 2, {end_col}, 15, {end_row}, 4</x:Anchor><x:AutoFill>False</x:AutoFill><x:Row>{row}</x:Row><x:Column>{col}</x:Column></x:ClientData></v:shape>"##,
            index + 1
        ));
    }
    Ok(s)
}

struct ParsedCommentsVml {
    note_cells: BTreeSet<(u32, u16)>,
    note_shapes: Vec<std::ops::Range<usize>>,
    root_end: usize,
    complete: bool,
}

struct VmlShapeScan {
    start: usize,
    depth: u32,
    is_note: bool,
    field: Option<VmlCellField>,
    row: String,
    col: String,
}

#[derive(Clone, Copy)]
enum VmlCellField {
    Row,
    Col,
}

fn reconcile_comments_vml(
    original: &[u8],
    cells: &BTreeSet<(u32, u16)>,
    shape_ids: &mut VmlShapeIdAllocator,
) -> Result<Vec<u8>, CoreError> {
    let parsed = parse_comments_vml(original)?;
    if parsed.complete && parsed.note_cells == *cells {
        return Ok(original.to_vec());
    }
    let generated = comments_vml_shapes(cells, shape_ids)?;
    let mut out = Vec::with_capacity(original.len().saturating_add(generated.len()));
    let mut cursor = 0usize;
    for shape in &parsed.note_shapes {
        if shape.start < cursor || shape.end > parsed.root_end {
            return Err(error::xlsx_write(
                "source VML note shape boundaries are invalid",
            ));
        }
        out.extend_from_slice(&original[cursor..shape.start]);
        cursor = shape.end;
    }
    out.extend_from_slice(&original[cursor..parsed.root_end]);
    out.extend_from_slice(generated.as_bytes());
    out.extend_from_slice(&original[parsed.root_end..]);
    Ok(out)
}

fn parse_comments_vml(bytes: &[u8]) -> Result<ParsedCommentsVml, CoreError> {
    let mut reader = xml::XmlReader::new(bytes);
    let mut depth = 0u32;
    let mut shape: Option<VmlShapeScan> = None;
    let mut note_cells = BTreeSet::new();
    let mut note_shapes = Vec::new();
    let mut root_end = None;
    let mut complete = true;
    while let Some(event) = reader.next()? {
        let span = reader.last_span();
        match event {
            xml::XmlEvent::Start { name, attrs } => {
                if shape.is_none() && name == "shape" {
                    shape = Some(VmlShapeScan {
                        start: span.start,
                        depth,
                        is_note: false,
                        field: None,
                        row: String::new(),
                        col: String::new(),
                    });
                }
                if let Some(shape) = &mut shape {
                    if name == "ClientData" && xml::attr(&attrs, "ObjectType") == Some("Note") {
                        shape.is_note = true;
                    } else if name == "Row" {
                        shape.field = Some(VmlCellField::Row);
                    } else if name == "Column" {
                        shape.field = Some(VmlCellField::Col);
                    }
                }
                depth = depth.saturating_add(1);
            }
            xml::XmlEvent::Empty { name, attrs } => {
                if let Some(shape) = &mut shape
                    && name == "ClientData"
                    && xml::attr(&attrs, "ObjectType") == Some("Note")
                {
                    shape.is_note = true;
                }
            }
            xml::XmlEvent::Text(text) => {
                if let Some(shape) = &mut shape {
                    match shape.field {
                        Some(VmlCellField::Row) => shape.row.push_str(&text),
                        Some(VmlCellField::Col) => shape.col.push_str(&text),
                        None => {}
                    }
                }
            }
            xml::XmlEvent::End { name } => {
                if let Some(shape) = &mut shape
                    && (name == "Row" || name == "Column")
                {
                    shape.field = None;
                }
                if name == "shape"
                    && shape
                        .as_ref()
                        .is_some_and(|shape| depth == shape.depth.saturating_add(1))
                    && let Some(shape) = shape.take()
                    && shape.is_note
                {
                    note_shapes.push(shape.start..span.end);
                    match (shape.row.trim().parse(), shape.col.trim().parse()) {
                        (Ok(row), Ok(col)) if row < MAX_ROWS && col < MAX_COLS => {
                            if !note_cells.insert((row, col)) {
                                complete = false;
                            }
                        }
                        _ => complete = false,
                    }
                }
                if name == "xml" && depth == 1 {
                    root_end = Some(span.start);
                }
                depth = depth.saturating_sub(1);
            }
        }
    }
    let root_end = root_end.ok_or_else(|| error::xlsx_write("source VML has no root end tag"))?;
    Ok(ParsedCommentsVml {
        note_cells,
        note_shapes,
        root_end,
        complete,
    })
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
            r#"<Relationship Id="{id}" Type="{}" Target="{}"{mode}/>"#,
            xml::escape(ty),
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
