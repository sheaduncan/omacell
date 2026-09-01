//! Pivot cache and table OPC parts (WP-24).

use std::collections::BTreeMap;

use omacell_core::addr::{SheetId, col_to_letters, parse_a1};
use omacell_core::error::CoreError;
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use omacell_core::pivot::{
    CacheValue, DateGroup, PivotAgg, PivotCalcField, PivotDataField, PivotGroup, PivotLayout,
    PivotTable, ShowAs, cache_table,
};
use omacell_core::workbook::Workbook;

use super::opc::{OpcPackage, Relationship};
use super::warnings::FileWarnings;
use super::xml::{XmlEvent, XmlReader, attr, escape};
use crate::error;

pub(crate) const REL_PIVOT_CACHE_DEF: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotCacheDefinition";
pub(crate) const REL_PIVOT_CACHE_REC: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotCacheRecords";
pub(crate) const REL_PIVOT_TABLE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotTable";
pub(crate) const CT_PIVOT_CACHE_DEF: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml";
pub(crate) const CT_PIVOT_CACHE_REC: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheRecords+xml";
pub(crate) const CT_PIVOT_TABLE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml";

const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_PKG: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const NS_X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const X14_CACHE_URI: &str = "{725AE2AE-9491-48be-BD36-2398A43DD21F}";
const X14_DATA_URI: &str = "{E15A36E0-9728-4e99-A89B-3F7291B0FE68}";
const MAX_PIVOT_CACHE_CELLS: usize = 1_000_000;

pub(crate) fn cache_id_of(pivot: &PivotTable) -> u32 {
    pivot
        .ooxml_cache_id
        .unwrap_or_else(|| pivot.id.index().saturating_add(1).max(1))
}

fn cache_def_name(pivot: &PivotTable) -> String {
    pivot.ooxml_cache_def.clone().unwrap_or_else(|| {
        format!(
            "xl/pivotCache/pivotCacheDefinition{}.xml",
            cache_id_of(pivot)
        )
    })
}

fn table_part_name(pivot: &PivotTable) -> String {
    pivot
        .ooxml_table
        .clone()
        .unwrap_or_else(|| format!("xl/pivotTables/pivotTable{}.xml", cache_id_of(pivot)))
}

/// Cache definition + records parts for one pivot.
pub(crate) struct CacheParts {
    /// OOXML cacheId (1-based).
    pub cache_id: u32,
    /// Workbook-relative cache definition target.
    pub def_target: String,
    /// Parts `(name, bytes, content_type)`.
    pub parts: Vec<(String, Vec<u8>, String)>,
}

/// Worksheet pivotTable part + its rels.
pub(crate) struct TableParts {
    /// Worksheet relationship target (`../pivotTables/pivotTable1.xml`).
    pub rel_target: String,
    /// Extra OPC parts.
    pub parts: Vec<(String, Vec<u8>, String)>,
}

/// Copy original cache parts when the modeled pivot is unchanged.
pub(crate) fn preserved_cache_parts(
    package: &OpcPackage,
    pivot: &PivotTable,
) -> Option<CacheParts> {
    if pivot.ooxml_dirty {
        return None;
    }
    let def_name = pivot.ooxml_cache_def.as_deref()?;
    let mut parts = Vec::new();
    copy_part_tree(package, def_name, &mut parts)?;
    Some(CacheParts {
        cache_id: cache_id_of(pivot),
        def_target: workbook_rel_from_part(def_name),
        parts,
    })
}

/// Copy original pivot table parts when the modeled pivot is unchanged.
pub(crate) fn preserved_table_parts(
    package: &OpcPackage,
    pivot: &PivotTable,
) -> Option<TableParts> {
    if pivot.ooxml_dirty {
        return None;
    }
    let table_name = pivot.ooxml_table.as_deref()?;
    let mut parts = Vec::new();
    copy_part_tree(package, table_name, &mut parts)?;
    Some(TableParts {
        rel_target: sheet_rel_from_part(table_name),
        parts,
    })
}

fn workbook_rel_from_part(name: &str) -> String {
    name.strip_prefix("xl/")
        .or_else(|| name.strip_prefix("/xl/"))
        .unwrap_or(name)
        .to_string()
}

fn sheet_rel_from_part(name: &str) -> String {
    if let Some(rest) = name.strip_prefix("xl/") {
        format!("../{rest}")
    } else {
        format!("../{name}")
    }
}

fn copy_part_tree(
    package: &OpcPackage,
    name: &str,
    out: &mut Vec<(String, Vec<u8>, String)>,
) -> Option<()> {
    if out
        .iter()
        .any(|(existing, _, _)| existing.eq_ignore_ascii_case(name))
    {
        return Some(());
    }
    let part = package.part(name)?;
    out.push((
        part.name.clone(),
        part.bytes.clone(),
        part.content_type.clone().unwrap_or_default(),
    ));
    let rels_name = super::opc::rels_path(name);
    if let Some(rels_part) = package.part(&rels_name) {
        out.push((
            rels_part.name.clone(),
            rels_part.bytes.clone(),
            rels_part.content_type.clone().unwrap_or_default(),
        ));
    }
    if let Ok(rels) = package.rels_for(name) {
        for rel in rels {
            if rel.external {
                continue;
            }
            let target = rel.target;
            let lower = target.to_ascii_lowercase();
            if lower.starts_with("xl/workbook") || lower.starts_with("xl/worksheets/") {
                continue;
            }
            copy_part_tree(package, &target, out)?;
        }
    }
    Some(())
}

/// Emit cache definition and records for `pivot`.
pub(crate) fn cache_parts(wb: &Workbook, pivot: &PivotTable) -> Result<CacheParts, CoreError> {
    let cache_id = cache_id_of(pivot);
    let (headers, rows) = cache_table(wb, pivot).map_err(|e| error::xlsx_write(e.to_string()))?;
    let def_name = cache_def_name(pivot);
    let rec_name = {
        let stem = def_name
            .rsplit('/')
            .next()
            .unwrap_or("pivotCacheDefinition1.xml");
        let rec = stem.replace("Definition", "Records");
        match def_name.rsplit_once('/') {
            Some((dir, _)) => format!("{dir}/{rec}"),
            None => rec,
        }
    };
    let rels_name = super::opc::rels_path(&def_name);
    let sheet_name = wb
        .sheet(pivot.source_sheet)
        .map(|s| s.name.as_str())
        .unwrap_or("Sheet1");
    let source_ref = a1_range(
        pivot.source.start.row.min(pivot.source.end.row),
        pivot.source.start.col.min(pivot.source.end.col),
        pivot.source.start.row.max(pivot.source.end.row),
        pivot.source.start.col.max(pivot.source.end.col),
    )?;
    let refresh = u8::from(pivot.refresh_on_load);
    let mut fields = String::new();
    for (i, header) in headers.iter().enumerate() {
        let values: Vec<&CacheValue> = rows.iter().filter_map(|row| row.get(i)).collect();
        let (shared, attrs) = shared_items(&values);
        let group = pivot
            .groups
            .get(header)
            .map(field_group_xml)
            .unwrap_or_default();
        let calc = pivot
            .calc_fields
            .iter()
            .find(|field| field.name == *header)
            .map(|field| format!(r#" databaseField="0" formula="{}""#, escape(&field.formula)))
            .unwrap_or_default();
        fields.push_str(&format!(
            r#"<cacheField name="{}" numFmtId="0"{calc}><sharedItems{attrs}>{shared}</sharedItems>{group}</cacheField>"#,
            escape(header)
        ));
    }
    let distinct = pivot
        .data
        .iter()
        .any(|field| field.agg == PivotAgg::DistinctCount);
    let x14 = if distinct {
        format!(
            r#" xmlns:x14="{NS_X14}"><cacheSource type="worksheet"><worksheetSource ref="{source_ref}" sheet="{}"/></cacheSource><cacheFields count="{}">{fields}</cacheFields><extLst><ext uri="{X14_CACHE_URI}"><x14:pivotCacheDefinition/></ext></extLst></pivotCacheDefinition>"#,
            escape(sheet_name),
            headers.len()
        )
    } else {
        format!(
            r#"><cacheSource type="worksheet"><worksheetSource ref="{source_ref}" sheet="{}"/></cacheSource><cacheFields count="{}">{fields}</cacheFields></pivotCacheDefinition>"#,
            escape(sheet_name),
            headers.len()
        )
    };
    let def = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><pivotCacheDefinition xmlns="{NS}" xmlns:r="{NS_R}" r:id="rId1" refreshOnLoad="{refresh}" recordCount="{}" createdVersion="8" refreshedVersion="8"{x14}"#,
        rows.len(),
    );
    let mut recs = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><pivotCacheRecords xmlns="{NS}" count="{}">"#,
        rows.len()
    );
    for row in &rows {
        recs.push_str("<r>");
        for value in row {
            match value {
                CacheValue::Number(n) => recs.push_str(&format!(r#"<n v="{n}"/>"#)),
                CacheValue::Text(t) => recs.push_str(&format!(r#"<s v="{}"/>"#, escape(t))),
                CacheValue::Empty => recs.push_str("<m/>"),
            }
        }
        recs.push_str("</r>");
    }
    recs.push_str("</pivotCacheRecords>");
    let rec_file = rec_name
        .rsplit('/')
        .next()
        .unwrap_or("pivotCacheRecords1.xml");
    let rels = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="{NS_PKG}"><Relationship Id="rId1" Type="{REL_PIVOT_CACHE_REC}" Target="{rec_file}"/></Relationships>"#
    );
    Ok(CacheParts {
        cache_id,
        def_target: workbook_rel_from_part(&def_name),
        parts: vec![
            (def_name, def.into_bytes(), CT_PIVOT_CACHE_DEF.into()),
            (rec_name, recs.into_bytes(), CT_PIVOT_CACHE_REC.into()),
            (rels_name, rels.into_bytes(), String::new()),
        ],
    })
}

/// Emit a pivotTable part related from the destination worksheet.
pub(crate) fn table_parts(wb: &Workbook, pivot: &PivotTable) -> Result<TableParts, CoreError> {
    let cache_id = cache_id_of(pivot);
    let name = table_part_name(pivot);
    let rels_name = super::opc::rels_path(&name);
    let (headers, rows) = cache_table(wb, pivot).map_err(|e| error::xlsx_write(e.to_string()))?;
    let shared: Vec<Vec<CacheValue>> = (0..headers.len())
        .map(|index| {
            let values: Vec<&CacheValue> = rows.iter().filter_map(|row| row.get(index)).collect();
            unique_items(&values)
        })
        .collect();
    let loc = a1_range(
        pivot.dest_row,
        pivot.dest_col,
        pivot.out_end_row.max(pivot.dest_row),
        pivot.out_end_col.max(pivot.dest_col),
    )?;
    let compact = u8::from(matches!(pivot.layout, PivotLayout::Compact));
    let outline = u8::from(matches!(pivot.layout, PivotLayout::Outline));
    let mut fields = String::new();
    for (index, header) in headers.iter().enumerate() {
        let axis = if pivot.rows.iter().any(|n| n == header) {
            r#" axis="axisRow""#
        } else if pivot.cols.iter().any(|n| n == header) {
            r#" axis="axisCol""#
        } else if pivot.filters.iter().any(|(n, _)| n == header) {
            r#" axis="axisPage""#
        } else if pivot.data.iter().any(|d| d.source == *header) {
            r#" dataField="1""#
        } else {
            ""
        };
        let no_subtotals = if !pivot.subtotals && pivot.rows.iter().any(|name| name == header) {
            r#" defaultSubtotal="0""#
        } else {
            ""
        };
        let filter = pivot.filters.iter().find(|(name, _)| name == header);
        let multiple = if filter.is_some_and(|(_, allowed)| allowed.len() > 1) {
            r#" multipleItemSelectionAllowed="1""#
        } else {
            ""
        };
        let items = if axis.is_empty() || axis.contains("dataField") {
            String::new()
        } else {
            let values = shared.get(index).map(Vec::as_slice).unwrap_or(&[]);
            let mut xml = String::new();
            for (item, value) in values.iter().enumerate() {
                let hidden = filter
                    .filter(|(_, allowed)| !allowed.is_empty())
                    .is_some_and(|(_, allowed)| !allowed.contains(&cache_value_text(value)));
                let hidden_attr = if hidden { r#" h="1""# } else { "" };
                xml.push_str(&format!(r#"<item x="{item}"{hidden_attr}/>"#));
            }
            xml.push_str(r#"<item t="default"/>"#);
            format!(r#"<items count="{}">{xml}</items>"#, values.len() + 1)
        };
        fields.push_str(&format!(
            r#"<pivotField{axis}{no_subtotals}{multiple} showAll="0">{items}</pivotField>"#
        ));
    }
    let mut row_fields = String::new();
    for name in &pivot.rows {
        if let Some(i) = headers.iter().position(|h| h == name) {
            row_fields.push_str(&format!(r#"<field x="{i}"/>"#));
        }
    }
    let mut col_fields = String::new();
    for name in &pivot.cols {
        if let Some(i) = headers.iter().position(|h| h == name) {
            col_fields.push_str(&format!(r#"<field x="{i}"/>"#));
        }
    }
    let mut page_fields = String::new();
    for (name, allowed) in &pivot.filters {
        if let Some(i) = headers.iter().position(|h| h == name) {
            let selected = if allowed.len() == 1 {
                shared
                    .get(i)
                    .and_then(|items| {
                        items
                            .iter()
                            .position(|value| cache_value_text(value) == allowed[0])
                    })
                    .map(|item| format!(r#" item="{item}""#))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            page_fields.push_str(&format!(r#"<pageField fld="{i}" hier="-1"{selected}/>"#));
        }
    }
    let mut data_fields = String::new();
    for df in &pivot.data {
        if let Some(i) = headers.iter().position(|h| h == &df.source) {
            let show = if df.show_as == ShowAs::Normal {
                String::new()
            } else {
                format!(r#" showDataAs="{}""#, df.show_as.ooxml())
            };
            let name = format!("{} {}", agg_caption(df.agg), df.source);
            if df.agg == PivotAgg::DistinctCount {
                data_fields.push_str(&format!(
                    r#"<dataField name="{}" fld="{i}" subtotal="{}"{show}><extLst><ext uri="{X14_DATA_URI}"><x14:dataField pivotShowAs="distinctCount"/></ext></extLst></dataField>"#,
                    escape(&name),
                    df.agg.ooxml_subtotal()
                ));
            } else {
                data_fields.push_str(&format!(
                    r#"<dataField name="{}" fld="{i}" subtotal="{}"{show}/>"#,
                    escape(&name),
                    df.agg.ooxml_subtotal()
                ));
            }
        }
    }
    let row_xml = if pivot.rows.is_empty() {
        String::new()
    } else {
        format!(
            r#"<rowFields count="{}">{row_fields}</rowFields>"#,
            pivot.rows.len()
        )
    };
    let col_xml = if pivot.cols.is_empty() {
        String::new()
    } else {
        format!(
            r#"<colFields count="{}">{col_fields}</colFields>"#,
            pivot.cols.len()
        )
    };
    let page_xml = if pivot.filters.is_empty() {
        String::new()
    } else {
        format!(
            r#"<pageFields count="{}">{page_fields}</pageFields>"#,
            pivot.filters.len()
        )
    };
    let data_xml = if pivot.data.is_empty() {
        String::new()
    } else {
        format!(
            r#"<dataFields count="{}">{data_fields}</dataFields>"#,
            pivot.data.len()
        )
    };
    let x14_ns = if pivot
        .data
        .iter()
        .any(|field| field.agg == PivotAgg::DistinctCount)
    {
        format!(r#" xmlns:x14="{NS_X14}""#)
    } else {
        String::new()
    };
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><pivotTableDefinition xmlns="{NS}"{x14_ns} name="{}" cacheId="{cache_id}" dataCaption="Values" rowGrandTotals="{}" colGrandTotals="{}" compact="{compact}" compactData="{compact}" outline="{outline}" outlineData="{outline}"><location ref="{loc}" firstHeaderRow="1" firstDataRow="1" firstDataCol="1"/><pivotFields count="{}">{fields}</pivotFields>{row_xml}{col_xml}{page_xml}{data_xml}</pivotTableDefinition>"#,
        escape(&pivot.name),
        u8::from(pivot.grand_rows),
        u8::from(pivot.grand_cols),
        headers.len()
    );
    let cache_target = {
        let def = cache_def_name(pivot);
        if let Some(rest) = def.strip_prefix("xl/") {
            format!("../{rest}")
        } else {
            format!("../{def}")
        }
    };
    let rels = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="{NS_PKG}"><Relationship Id="rId1" Type="{REL_PIVOT_CACHE_DEF}" Target="{cache_target}"/></Relationships>"#
    );
    Ok(TableParts {
        rel_target: sheet_rel_from_part(&name),
        parts: vec![
            (name, xml.into_bytes(), CT_PIVOT_TABLE.into()),
            (rels_name, rels.into_bytes(), String::new()),
        ],
    })
}

/// Parsed pivot cache used while loading worksheet pivot tables.
pub(crate) struct LoadedCache {
    source_sheet: String,
    source_ref: String,
    headers: Vec<String>,
    shared: Vec<Vec<CacheValue>>,
    rows: Vec<Vec<CacheValue>>,
    groups: BTreeMap<String, PivotGroup>,
    calc_fields: Vec<PivotCalcField>,
    refresh_on_load: bool,
    def_part: String,
}

/// Load pivot caches declared on `workbook.xml`.
pub(crate) fn load_caches(
    package: &OpcPackage,
    wb_name: &str,
    wb_rels: &[Relationship],
    warnings: &mut FileWarnings,
) -> Result<BTreeMap<u32, LoadedCache>, CoreError> {
    let mut out = BTreeMap::new();
    let Some(wb_part) = package.part(wb_name) else {
        return Ok(out);
    };
    let mut r = XmlReader::new(&wb_part.bytes);
    let mut cache_ids: Vec<(u32, String)> = Vec::new();
    while let Some(ev) = r.next()? {
        match ev {
            XmlEvent::Start { name, attrs } | XmlEvent::Empty { name, attrs }
                if name == "pivotCache" =>
            {
                let id = attr(&attrs, "cacheId")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let rid = attr(&attrs, "id").unwrap_or("").to_string();
                cache_ids.push((id, rid));
                if cache_ids.len() > usize::from(MAX_COLS) {
                    return Err(error::xlsx_limit(format!(
                        "workbook has more than {MAX_COLS} pivot caches"
                    )));
                }
            }
            _ => {}
        }
    }
    for (cache_id, rid) in cache_ids {
        let Some(rel) = wb_rels.iter().find(|r| r.id == rid) else {
            warnings.push(
                "xlsx.pivot",
                format!("pivot cache {cache_id} has no relationship"),
                Some(wb_name.into()),
            );
            continue;
        };
        match parse_cache(package, rel) {
            Ok(cache) => {
                out.insert(cache_id, cache);
            }
            Err(error) => warnings.push("xlsx.pivot", error.message, Some(rel.target.clone())),
        }
    }
    Ok(out)
}

fn parse_cache(package: &OpcPackage, rel: &Relationship) -> Result<LoadedCache, CoreError> {
    let Some(part) = package.part(&rel.target) else {
        return Err(error::xlsx_format("pivot cache definition missing"));
    };
    let mut r = XmlReader::new(&part.bytes);
    let mut source_sheet = String::new();
    let mut source_ref = String::new();
    let mut headers = Vec::new();
    let mut groups = BTreeMap::new();
    let mut shared: Vec<Vec<CacheValue>> = Vec::new();
    let mut shared_item_count = 0usize;
    let mut refresh_on_load = false;
    let mut current_field = String::new();
    let mut current_field_index = None;
    let mut in_shared_items = false;
    let mut calc_fields = Vec::new();
    while let Some(ev) = r.next()? {
        match ev {
            XmlEvent::Start { name, attrs } | XmlEvent::Empty { name, attrs }
                if name == "pivotCacheDefinition" =>
            {
                refresh_on_load = attr(&attrs, "refreshOnLoad").is_some_and(|s| s != "0");
            }
            XmlEvent::Start { name, attrs } | XmlEvent::Empty { name, attrs }
                if name == "worksheetSource" =>
            {
                source_sheet = attr(&attrs, "sheet").unwrap_or("").to_string();
                source_ref = attr(&attrs, "ref").unwrap_or("").to_string();
            }
            XmlEvent::Start { name, attrs } | XmlEvent::Empty { name, attrs }
                if name == "cacheField" =>
            {
                if headers.len() >= usize::from(MAX_COLS) {
                    return Err(error::xlsx_limit(format!(
                        "pivot cache has more than {MAX_COLS} fields"
                    )));
                }
                current_field = attr(&attrs, "name").unwrap_or("Field").to_string();
                headers.push(current_field.clone());
                shared.push(Vec::new());
                current_field_index = Some(headers.len() - 1);
                let calculated = attr(&attrs, "databaseField").is_some_and(|value| value == "0");
                if calculated {
                    calc_fields.push(PivotCalcField {
                        name: current_field.clone(),
                        formula: attr(&attrs, "formula").unwrap_or("").to_string(),
                    });
                }
            }
            XmlEvent::Start { name, .. } if name == "sharedItems" => in_shared_items = true,
            XmlEvent::End { name } if name == "sharedItems" => in_shared_items = false,
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if in_shared_items =>
            {
                if let (Some(index), Some(value)) =
                    (current_field_index, parse_cache_value(&name, &attrs)?)
                    && let Some(items) = shared.get_mut(index)
                {
                    items.push(value);
                    shared_item_count = shared_item_count.saturating_add(1);
                    if shared_item_count > MAX_PIVOT_CACHE_CELLS {
                        return Err(error::xlsx_limit(format!(
                            "pivot cache has more than {MAX_PIVOT_CACHE_CELLS} shared items"
                        )));
                    }
                }
            }
            XmlEvent::Start { name, attrs } | XmlEvent::Empty { name, attrs }
                if name == "rangePr" =>
            {
                if let Some(group) = parse_range_pr(&attrs) {
                    groups.insert(current_field.clone(), group);
                }
            }
            _ => {}
        }
    }
    if headers.len() > usize::from(MAX_COLS) {
        return Err(error::xlsx_limit(format!(
            "pivot cache has {} fields; maximum is {MAX_COLS}",
            headers.len()
        )));
    }
    let recs = package
        .rels_for(&rel.target)?
        .into_iter()
        .find(|r| r.rel_type == REL_PIVOT_CACHE_REC);
    let rows = if let Some(rec_rel) = recs {
        parse_records(package, &rec_rel, headers.len(), &shared)?
    } else {
        Vec::new()
    };
    Ok(LoadedCache {
        source_sheet,
        source_ref,
        headers,
        shared,
        rows,
        groups,
        calc_fields,
        refresh_on_load,
        def_part: rel.target.clone(),
    })
}

fn parse_records(
    package: &OpcPackage,
    rel: &Relationship,
    width: usize,
    shared: &[Vec<CacheValue>],
) -> Result<Vec<Vec<CacheValue>>, CoreError> {
    let Some(part) = package.part(&rel.target) else {
        return Ok(Vec::new());
    };
    let mut r = XmlReader::new(&part.bytes);
    let mut rows = Vec::new();
    let mut current: Option<Vec<CacheValue>> = None;
    while let Some(ev) = r.next()? {
        match ev {
            XmlEvent::Start { name, .. } if name == "r" => {
                current = Some(Vec::new());
            }
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs } => {
                if let Some(row) = current.as_mut() {
                    if row.len() >= width {
                        return Err(error::xlsx_limit(
                            "pivot cache record is wider than its field list",
                        ));
                    }
                    if name == "x" {
                        let item = attr(&attrs, "v")
                            .and_then(|value| value.parse::<usize>().ok())
                            .ok_or_else(|| {
                                error::xlsx_format("pivot cache shared-item index is invalid")
                            })?;
                        let field = row.len();
                        let value = shared
                            .get(field)
                            .and_then(|items| items.get(item))
                            .cloned()
                            .ok_or_else(|| {
                                error::xlsx_format(
                                    "pivot cache record references a missing shared item",
                                )
                            })?;
                        row.push(value);
                    } else if let Some(value) = parse_cache_value(&name, &attrs)? {
                        row.push(value);
                    }
                }
            }
            XmlEvent::End { name } if name == "r" => {
                if let Some(mut row) = current.take() {
                    row.resize(width, CacheValue::Empty);
                    let cells = rows
                        .len()
                        .checked_add(1)
                        .and_then(|count| count.checked_mul(width))
                        .ok_or_else(|| error::xlsx_limit("pivot cache size overflows"))?;
                    if rows.len() >= MAX_ROWS as usize || cells > MAX_PIVOT_CACHE_CELLS {
                        return Err(error::xlsx_limit(format!(
                            "pivot cache has more than {MAX_PIVOT_CACHE_CELLS} cells"
                        )));
                    }
                    rows.push(row);
                }
            }
            _ => {}
        }
    }
    Ok(rows)
}

fn parse_cache_value(
    name: &str,
    attrs: &[(String, String)],
) -> Result<Option<CacheValue>, CoreError> {
    let value = match name {
        "n" => {
            let number = attr(attrs, "v")
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|value| value.is_finite())
                .ok_or_else(|| error::xlsx_format("pivot cache number is invalid"))?;
            CacheValue::Number(number)
        }
        "s" | "d" | "e" => CacheValue::Text(attr(attrs, "v").unwrap_or("").to_string()),
        "b" => CacheValue::Number(if attr(attrs, "v").is_some_and(|value| value != "0") {
            1.0
        } else {
            0.0
        }),
        "m" => CacheValue::Empty,
        _ => return Ok(None),
    };
    Ok(Some(value))
}

/// Load pivot tables related from one worksheet.
pub(crate) fn load_sheet_pivots(
    wb: &mut Workbook,
    package: &OpcPackage,
    sheet: SheetId,
    sheet_rels: &[Relationship],
    caches: &BTreeMap<u32, LoadedCache>,
    warnings: &mut FileWarnings,
) -> Result<(), CoreError> {
    for rel in sheet_rels.iter().filter(|r| r.rel_type == REL_PIVOT_TABLE) {
        if let Err(error) = load_table(wb, package, rel, sheet, caches, warnings) {
            warnings.push("xlsx.pivot", error.message, Some(rel.target.clone()));
        }
    }
    Ok(())
}

fn load_table(
    wb: &mut Workbook,
    package: &OpcPackage,
    rel: &Relationship,
    dest_sheet: SheetId,
    caches: &BTreeMap<u32, LoadedCache>,
    warnings: &mut FileWarnings,
) -> Result<(), CoreError> {
    let Some(part) = package.part(&rel.target) else {
        return Err(error::xlsx_format("pivot table part missing"));
    };
    let mut r = XmlReader::new(&part.bytes);
    let mut name = String::from("Pivot");
    let mut cache_id = 0u32;
    let mut loc = String::new();
    let mut grand_rows = true;
    let mut grand_cols = true;
    let mut layout = PivotLayout::Compact;
    let mut headers: Vec<String> = Vec::new();
    let mut row_idx: Vec<usize> = Vec::new();
    let mut col_idx: Vec<usize> = Vec::new();
    let mut page_specs: Vec<(usize, Option<usize>)> = Vec::new();
    let mut data = Vec::new();
    let mut current_data: Option<(usize, PivotAgg, ShowAs)> = None;
    let mut pivot_field_items: Vec<Vec<(usize, bool)>> = Vec::new();
    let mut pivot_item_count = 0usize;
    let mut current_pivot_field = None;
    let mut no_subtotal_fields = std::collections::BTreeSet::new();
    let mut in_rows = false;
    let mut in_cols = false;
    let mut in_pages = false;
    while let Some(ev) = r.next()? {
        match ev {
            XmlEvent::Start { name: n, attrs } | XmlEvent::Empty { name: n, attrs }
                if n == "pivotTableDefinition" =>
            {
                name = attr(&attrs, "name").unwrap_or("Pivot").to_string();
                cache_id = attr(&attrs, "cacheId")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                grand_rows = attr(&attrs, "rowGrandTotals").is_none_or(|s| s != "0");
                grand_cols = attr(&attrs, "colGrandTotals").is_none_or(|s| s != "0");
                let compact = attr(&attrs, "compact").is_none_or(|s| s != "0");
                let outline = attr(&attrs, "outline").is_some_and(|s| s != "0");
                layout = if compact {
                    PivotLayout::Compact
                } else if outline {
                    PivotLayout::Outline
                } else {
                    PivotLayout::Tabular
                };
            }
            XmlEvent::Start { name: n, attrs } | XmlEvent::Empty { name: n, attrs }
                if n == "location" =>
            {
                loc = attr(&attrs, "ref").unwrap_or("").to_string();
            }
            XmlEvent::Start { name: n, attrs } if n == "pivotField" => {
                if pivot_field_items.len() >= usize::from(MAX_COLS) {
                    return Err(error::xlsx_limit(format!(
                        "pivot table has more than {MAX_COLS} fields"
                    )));
                }
                let index = pivot_field_items.len();
                if attr(&attrs, "defaultSubtotal").is_some_and(|value| value == "0") {
                    no_subtotal_fields.insert(index);
                }
                pivot_field_items.push(Vec::new());
                current_pivot_field = Some(index);
            }
            XmlEvent::Empty { name: n, attrs } if n == "pivotField" => {
                if pivot_field_items.len() >= usize::from(MAX_COLS) {
                    return Err(error::xlsx_limit(format!(
                        "pivot table has more than {MAX_COLS} fields"
                    )));
                }
                let index = pivot_field_items.len();
                if attr(&attrs, "defaultSubtotal").is_some_and(|value| value == "0") {
                    no_subtotal_fields.insert(index);
                }
                pivot_field_items.push(Vec::new());
                current_pivot_field = None;
            }
            XmlEvent::End { name: n } if n == "pivotField" => current_pivot_field = None,
            XmlEvent::Empty { name: n, attrs } | XmlEvent::Start { name: n, attrs }
                if n == "item" && current_pivot_field.is_some() =>
            {
                if let Some(shared_index) = attr(&attrs, "x").and_then(|value| value.parse().ok())
                    && let Some(items) =
                        current_pivot_field.and_then(|index| pivot_field_items.get_mut(index))
                {
                    let hidden = attr(&attrs, "h").is_some_and(|value| value != "0");
                    items.push((shared_index, hidden));
                    pivot_item_count = pivot_item_count.saturating_add(1);
                    if pivot_item_count > MAX_PIVOT_CACHE_CELLS {
                        return Err(error::xlsx_limit(format!(
                            "pivot table has more than {MAX_PIVOT_CACHE_CELLS} items"
                        )));
                    }
                }
            }
            XmlEvent::Start { name: n, .. } if n == "rowFields" => in_rows = true,
            XmlEvent::End { name: n } if n == "rowFields" => in_rows = false,
            XmlEvent::Start { name: n, .. } if n == "colFields" => in_cols = true,
            XmlEvent::End { name: n } if n == "colFields" => in_cols = false,
            XmlEvent::Start { name: n, .. } if n == "pageFields" => in_pages = true,
            XmlEvent::End { name: n } if n == "pageFields" => in_pages = false,
            XmlEvent::Empty { name: n, attrs } | XmlEvent::Start { name: n, attrs }
                if n == "field" && in_rows =>
            {
                if let Some(x) = attr(&attrs, "x").and_then(|s| s.parse().ok()) {
                    row_idx.push(x);
                    if row_idx.len() > usize::from(MAX_COLS) {
                        return Err(error::xlsx_limit("too many pivot row fields"));
                    }
                }
            }
            XmlEvent::Empty { name: n, attrs } | XmlEvent::Start { name: n, attrs }
                if n == "field" && in_cols =>
            {
                if let Some(x) = attr(&attrs, "x").and_then(|s| s.parse().ok()) {
                    col_idx.push(x);
                    if col_idx.len() > usize::from(MAX_COLS) {
                        return Err(error::xlsx_limit("too many pivot column fields"));
                    }
                }
            }
            XmlEvent::Empty { name: n, attrs } | XmlEvent::Start { name: n, attrs }
                if n == "pageField" && in_pages =>
            {
                if let Some(x) = attr(&attrs, "fld").and_then(|s| s.parse().ok()) {
                    let item = attr(&attrs, "item").and_then(|value| value.parse().ok());
                    page_specs.push((x, item));
                    if page_specs.len() > usize::from(MAX_COLS) {
                        return Err(error::xlsx_limit("too many pivot page fields"));
                    }
                }
            }
            XmlEvent::Empty { name: n, attrs } if n == "dataField" => {
                if let Some((_, agg, _)) = current_data.as_mut() {
                    if attr(&attrs, "pivotShowAs").is_some_and(|value| value == "distinctCount") {
                        *agg = PivotAgg::DistinctCount;
                    }
                } else {
                    data.push(parse_data_field(&attrs));
                    if data.len() > usize::from(MAX_COLS) {
                        return Err(error::xlsx_limit("too many pivot data fields"));
                    }
                }
            }
            XmlEvent::Start { name: n, attrs } if n == "dataField" => {
                if current_data.is_some() {
                    if attr(&attrs, "pivotShowAs").is_some_and(|value| value == "distinctCount")
                        && let Some((_, agg, _)) = current_data.as_mut()
                    {
                        *agg = PivotAgg::DistinctCount;
                    }
                } else {
                    current_data = Some(parse_data_field(&attrs));
                }
            }
            XmlEvent::End { name: n } if n == "dataField" => {
                if let Some(field) = current_data.take() {
                    data.push(field);
                    if data.len() > usize::from(MAX_COLS) {
                        return Err(error::xlsx_limit("too many pivot data fields"));
                    }
                }
            }
            _ => {}
        }
    }
    let cache = caches.get(&cache_id);
    if let Some(cache) = cache {
        headers = cache.headers.clone();
    }
    let Ok(parsed) = parse_a1(&loc) else {
        return Err(error::xlsx_format("pivot location is not a valid A1 range"));
    };
    let (dr0, dc0, dr1, dc1) = match parsed.kind {
        omacell_core::addr::RefKind::Range(rg) => (
            rg.start.row.min(rg.end.row),
            rg.start.col.min(rg.end.col),
            rg.start.row.max(rg.end.row),
            rg.start.col.max(rg.end.col),
        ),
        omacell_core::addr::RefKind::Cell(c) => (c.row, c.col, c.row, c.col),
    };
    let source_sheet_found = cache.and_then(|c| {
        wb.sheets()
            .find(|s| s.name.eq_ignore_ascii_case(&c.source_sheet))
            .map(|s| s.id)
    });
    let source_sheet = source_sheet_found.unwrap_or(dest_sheet);
    let source_ref = cache.map(|c| c.source_ref.as_str()).unwrap_or("A1");
    let source = match parse_a1(source_ref) {
        Ok(p) => match p.kind {
            omacell_core::addr::RefKind::Range(rg) => rg,
            omacell_core::addr::RefKind::Cell(c) => {
                omacell_core::addr::RangeRef::from_corners(c, c)
            }
        },
        Err(_) => omacell_core::addr::RangeRef::from_corners(
            omacell_core::addr::CellRef::new(0, 0)?,
            omacell_core::addr::CellRef::new(0, 0)?,
        ),
    };
    let mut table = PivotTable::new(name, source_sheet, source, dest_sheet, dr0, dc0);
    table.out_end_row = dr1;
    table.out_end_col = dc1;
    table.layout = layout;
    table.grand_rows = grand_rows;
    table.grand_cols = grand_cols;
    table.subtotals = !row_idx
        .iter()
        .any(|index| no_subtotal_fields.contains(index));
    table.refresh_on_load = cache.is_some_and(|c| c.refresh_on_load);
    table.rows = row_idx
        .into_iter()
        .filter_map(|i| headers.get(i).cloned())
        .collect();
    table.cols = col_idx
        .into_iter()
        .filter_map(|i| headers.get(i).cloned())
        .collect();
    table.filters = page_specs
        .into_iter()
        .filter_map(|(field, selected)| {
            let name = headers.get(field)?.clone();
            let items = pivot_field_items
                .get(field)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let shared = cache
                .and_then(|cache| cache.shared.get(field))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let has_hidden = items.iter().any(|(_, hidden)| *hidden);
            let allowed = if has_hidden {
                items
                    .iter()
                    .filter(|(_, hidden)| !hidden)
                    .filter_map(|(index, _)| shared.get(*index))
                    .map(cache_value_text)
                    .collect()
            } else if let Some(selected) = selected {
                items
                    .get(selected)
                    .and_then(|(index, _)| shared.get(*index))
                    .map(cache_value_text)
                    .into_iter()
                    .collect()
            } else {
                Vec::new()
            };
            Some((name, allowed))
        })
        .collect();
    table.data = data
        .into_iter()
        .filter_map(|(fld, agg, show_as)| {
            headers.get(fld).cloned().map(|source| PivotDataField {
                source,
                agg,
                show_as,
            })
        })
        .collect();
    if let Some(cache) = cache {
        table.groups = cache.groups.clone();
        table.calc_fields = cache.calc_fields.clone();
        table.ooxml_cache_def = Some(cache.def_part.clone());
    }
    table.ooxml_cache_id = Some(cache_id);
    table.ooxml_table = Some(rel.target.clone());
    table.ooxml_dirty = false;
    let source_present = source_sheet_found.is_some()
        && wb
            .get(source_sheet, source.start.row, source.start.col)
            .ok()
            .flatten()
            .is_some();
    let refresh_on_load = table.refresh_on_load;
    let id = match wb.import_pivot(table) {
        Ok(id) => id,
        Err(error) => {
            warnings.push("xlsx.pivot", error.message, Some(rel.target.clone()));
            return Ok(());
        }
    };
    if refresh_on_load && source_present {
        if let Err(error) = wb.refresh_pivot(id) {
            warnings.push("xlsx.pivot", error.message, Some(rel.target.clone()));
        }
    } else if !source_present
        && let Some(cache) = cache
        && let Err(error) = wb.refresh_pivot_from_cache(id, &cache.headers, &cache.rows)
    {
        warnings.push("xlsx.pivot", error.message, Some(rel.target.clone()));
    }
    let _ = wb.set_pivot_ooxml_dirty(id, false);
    Ok(())
}

fn parse_data_field(attrs: &[(String, String)]) -> (usize, PivotAgg, ShowAs) {
    let fld = attr(attrs, "fld")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let agg = attr(attrs, "subtotal")
        .and_then(PivotAgg::parse)
        .unwrap_or(PivotAgg::Sum);
    let show_as = attr(attrs, "showDataAs")
        .and_then(ShowAs::parse)
        .unwrap_or(ShowAs::Normal);
    (fld, agg, show_as)
}

fn parse_range_pr(attrs: &[(String, String)]) -> Option<PivotGroup> {
    let by = attr(attrs, "groupBy")?;
    if let Some(grain) = DateGroup::parse(by) {
        return Some(PivotGroup::Date(grain));
    }
    if by == "range" {
        let start = attr(attrs, "startNum")?.parse().ok()?;
        let size = attr(attrs, "groupInterval")?.parse().ok()?;
        return Some(PivotGroup::Numeric { start, size });
    }
    None
}

fn shared_items(values: &[&CacheValue]) -> (String, String) {
    let mut has_num = false;
    let mut has_str = false;
    let unique = unique_items(values);
    for value in values {
        match value {
            CacheValue::Number(_) => {
                has_num = true;
            }
            CacheValue::Text(t) if !t.is_empty() => {
                has_str = true;
            }
            _ => {}
        }
    }
    let mut xml = String::new();
    for v in &unique {
        match v {
            CacheValue::Number(n) => xml.push_str(&format!(r#"<n v="{n}"/>"#)),
            CacheValue::Text(t) => xml.push_str(&format!(r#"<s v="{}"/>"#, escape(t))),
            CacheValue::Empty => {}
        }
    }
    let mut attrs = format!(
        r#" count="{}" containsString="{}" containsNumber="{}" containsSemiMixedTypes="0""#,
        unique.len(),
        u8::from(has_str),
        u8::from(has_num)
    );
    if has_num
        && !has_str
        && unique
            .iter()
            .all(|value| matches!(value, CacheValue::Number(n) if n.fract() == 0.0))
    {
        attrs.push_str(r#" containsInteger="1""#);
    }
    (xml, attrs)
}

fn unique_items(values: &[&CacheValue]) -> Vec<CacheValue> {
    let mut unique: BTreeMap<(u8, String), CacheValue> = BTreeMap::new();
    for value in values {
        match value {
            CacheValue::Number(number) if number.is_finite() => {
                unique.insert((0, number.to_string()), CacheValue::Number(*number));
            }
            CacheValue::Text(text) if !text.is_empty() => {
                unique.insert((1, text.clone()), CacheValue::Text(text.clone()));
            }
            CacheValue::Number(_) | CacheValue::Text(_) | CacheValue::Empty => {}
        }
    }
    unique.into_values().collect()
}

fn cache_value_text(value: &CacheValue) -> String {
    match value {
        CacheValue::Number(number) => number.to_string(),
        CacheValue::Text(text) => text.clone(),
        CacheValue::Empty => String::new(),
    }
}

fn field_group_xml(group: &PivotGroup) -> String {
    match group {
        PivotGroup::None => String::new(),
        PivotGroup::Date(g) => format!(
            r#"<fieldGroup><rangePr groupBy="{}"/></fieldGroup>"#,
            g.as_str()
        ),
        PivotGroup::Numeric { start, size } => format!(
            r#"<fieldGroup><rangePr groupBy="range" startNum="{start}" groupInterval="{size}"/></fieldGroup>"#
        ),
    }
}

fn agg_caption(agg: PivotAgg) -> &'static str {
    match agg {
        PivotAgg::Sum => "Sum of",
        PivotAgg::Count => "Count of",
        PivotAgg::Average => "Average of",
        PivotAgg::Min => "Min of",
        PivotAgg::Max => "Max of",
        PivotAgg::CountA => "CountA of",
        PivotAgg::DistinctCount => "Distinct count of",
        PivotAgg::Stdev => "Stdev of",
        PivotAgg::Var => "Var of",
    }
}

fn a1_range(r0: u32, c0: u16, r1: u32, c1: u16) -> Result<String, CoreError> {
    if r0 >= MAX_ROWS || r1 >= MAX_ROWS {
        return Err(error::xlsx_write("pivot row exceeds the worksheet grid"));
    }
    let row0 = r0
        .checked_add(1)
        .ok_or_else(|| error::xlsx_write("pivot row exceeds the worksheet grid"))?;
    let row1 = r1
        .checked_add(1)
        .ok_or_else(|| error::xlsx_write("pivot row exceeds the worksheet grid"))?;
    Ok(format!(
        "{}{}:{}{}",
        col_to_letters(c0).map_err(|e| error::xlsx_write(e.to_string()))?,
        row0,
        col_to_letters(c1).map_err(|e| error::xlsx_write(e.to_string()))?,
        row1
    ))
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::*;
    use crate::xlsx::PreservedPart;

    #[test]
    fn cache_records_resolve_shared_item_indexes() {
        let target = "xl/pivotCache/pivotCacheRecords1.xml";
        let xml = br#"<pivotCacheRecords><r><x v="1"/><x v="0"/></r></pivotCacheRecords>"#;
        let mut parts = IndexMap::new();
        parts.insert(
            target.to_string(),
            PreservedPart {
                name: target.to_string(),
                content_type: None,
                bytes: xml.to_vec(),
            },
        );
        let package = OpcPackage {
            parts,
            package_rels: Vec::new(),
        };
        let rel = Relationship {
            id: "rId1".into(),
            rel_type: REL_PIVOT_CACHE_REC.into(),
            target: target.into(),
            external: false,
        };
        let shared = vec![
            vec![
                CacheValue::Text("East".into()),
                CacheValue::Text("West".into()),
            ],
            vec![CacheValue::Number(10.0)],
        ];
        let rows = parse_records(&package, &rel, 2, &shared).unwrap();
        assert_eq!(
            rows,
            vec![vec![
                CacheValue::Text("West".into()),
                CacheValue::Number(10.0)
            ]]
        );
    }
}
