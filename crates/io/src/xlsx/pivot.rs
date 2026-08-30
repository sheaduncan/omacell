//! Pivot cache and table OPC parts (WP-24).

use std::collections::BTreeMap;

use omacell_core::addr::{SheetId, col_to_letters, parse_a1};
use omacell_core::error::CoreError;
use omacell_core::pivot::{
    CacheValue, DateGroup, PivotAgg, PivotDataField, PivotGroup, PivotLayout, PivotTable, ShowAs,
    cache_table,
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

/// Emit cache definition and records for `pivot`.
pub(crate) fn cache_parts(wb: &Workbook, pivot: &PivotTable) -> Result<CacheParts, CoreError> {
    let cache_id = pivot.id.index().saturating_add(1);
    let (headers, rows) = cache_table(wb, pivot).map_err(|e| error::xlsx_write(e.to_string()))?;
    let def_name = format!("xl/pivotCache/pivotCacheDefinition{cache_id}.xml");
    let rec_name = format!("xl/pivotCache/pivotCacheRecords{cache_id}.xml");
    let rels_name = format!("xl/pivotCache/_rels/pivotCacheDefinition{cache_id}.xml.rels");
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
        fields.push_str(&format!(
            r#"<cacheField name="{}" numFmtId="0"><sharedItems{attrs}>{shared}</sharedItems>{group}</cacheField>"#,
            escape(header)
        ));
    }
    let def = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><pivotCacheDefinition xmlns="{NS}" xmlns:r="{NS_R}" r:id="rId1" refreshOnLoad="{refresh}" recordCount="{}" createdVersion="8" refreshedVersion="8"><cacheSource type="worksheet"><worksheetSource ref="{source_ref}" sheet="{}"/></cacheSource><cacheFields count="{}">{fields}</cacheFields></pivotCacheDefinition>"#,
        rows.len(),
        escape(sheet_name),
        headers.len()
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
    let rels = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="{NS_PKG}"><Relationship Id="rId1" Type="{REL_PIVOT_CACHE_REC}" Target="pivotCacheRecords{cache_id}.xml"/></Relationships>"#
    );
    Ok(CacheParts {
        cache_id,
        def_target: format!("pivotCache/pivotCacheDefinition{cache_id}.xml"),
        parts: vec![
            (def_name, def.into_bytes(), CT_PIVOT_CACHE_DEF.into()),
            (rec_name, recs.into_bytes(), CT_PIVOT_CACHE_REC.into()),
            (rels_name, rels.into_bytes(), String::new()),
        ],
    })
}

/// Emit a pivotTable part related from the destination worksheet.
pub(crate) fn table_parts(wb: &Workbook, pivot: &PivotTable) -> Result<TableParts, CoreError> {
    let cache_id = pivot.id.index().saturating_add(1);
    let number = cache_id;
    let name = format!("xl/pivotTables/pivotTable{number}.xml");
    let rels_name = format!("xl/pivotTables/_rels/pivotTable{number}.xml.rels");
    let (headers, _) = cache_table(wb, pivot).map_err(|e| error::xlsx_write(e.to_string()))?;
    let loc = a1_range(
        pivot.dest_row,
        pivot.dest_col,
        pivot.out_end_row.max(pivot.dest_row),
        pivot.out_end_col.max(pivot.dest_col),
    )?;
    let compact = u8::from(matches!(pivot.layout, PivotLayout::Compact));
    let outline = u8::from(matches!(pivot.layout, PivotLayout::Outline));
    let mut fields = String::new();
    for header in &headers {
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
        fields.push_str(&format!(
            r#"<pivotField{axis} showAll="0"><items count="1"><item t="default"/></items></pivotField>"#
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
    for (name, _) in &pivot.filters {
        if let Some(i) = headers.iter().position(|h| h == name) {
            page_fields.push_str(&format!(r#"<pageField fld="{i}" hier="-1"/>"#));
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
            data_fields.push_str(&format!(
                r#"<dataField name="{} {}" fld="{i}" subtotal="{}"{show}/>"#,
                escape(agg_caption(df.agg)),
                escape(&df.source),
                df.agg.ooxml_subtotal()
            ));
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
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><pivotTableDefinition xmlns="{NS}" name="{}" cacheId="{cache_id}" dataCaption="Values" rowGrandTotals="{}" colGrandTotals="{}" compact="{compact}" compactData="{compact}" outline="{outline}" outlineData="{outline}"><location ref="{loc}" firstHeaderRow="1" firstDataRow="1" firstDataCol="1"/><pivotFields count="{}">{fields}</pivotFields>{row_xml}{col_xml}{page_xml}{data_xml}</pivotTableDefinition>"#,
        escape(&pivot.name),
        u8::from(pivot.grand_rows),
        u8::from(pivot.grand_cols),
        headers.len()
    );
    let rels = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="{NS_PKG}"><Relationship Id="rId1" Type="{REL_PIVOT_CACHE_DEF}" Target="../pivotCache/pivotCacheDefinition{cache_id}.xml"/></Relationships>"#
    );
    Ok(TableParts {
        rel_target: format!("../pivotTables/pivotTable{number}.xml"),
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
    rows: Vec<Vec<CacheValue>>,
    groups: BTreeMap<String, PivotGroup>,
    refresh_on_load: bool,
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
    let mut refresh_on_load = false;
    let mut current_field = String::new();
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
                current_field = attr(&attrs, "name").unwrap_or("Field").to_string();
                headers.push(current_field.clone());
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
    let recs = package
        .rels_for(&rel.target)?
        .into_iter()
        .find(|r| r.rel_type == REL_PIVOT_CACHE_REC);
    let rows = if let Some(rec_rel) = recs {
        parse_records(package, &rec_rel, headers.len())?
    } else {
        Vec::new()
    };
    Ok(LoadedCache {
        source_sheet,
        source_ref,
        headers,
        rows,
        groups,
        refresh_on_load,
    })
}

fn parse_records(
    package: &OpcPackage,
    rel: &Relationship,
    width: usize,
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
                    if name == "n" {
                        row.push(CacheValue::Number(
                            attr(&attrs, "v")
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(0.0),
                        ));
                    } else if name == "s" {
                        row.push(CacheValue::Text(
                            attr(&attrs, "v").unwrap_or("").to_string(),
                        ));
                    } else if name == "m" {
                        row.push(CacheValue::Empty);
                    }
                }
            }
            XmlEvent::End { name } if name == "r" => {
                if let Some(mut row) = current.take() {
                    row.resize(width, CacheValue::Empty);
                    rows.push(row);
                }
            }
            _ => {}
        }
    }
    Ok(rows)
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
    let mut page_idx: Vec<usize> = Vec::new();
    let mut data = Vec::new();
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
                }
            }
            XmlEvent::Empty { name: n, attrs } | XmlEvent::Start { name: n, attrs }
                if n == "field" && in_cols =>
            {
                if let Some(x) = attr(&attrs, "x").and_then(|s| s.parse().ok()) {
                    col_idx.push(x);
                }
            }
            XmlEvent::Empty { name: n, attrs } | XmlEvent::Start { name: n, attrs }
                if n == "pageField" && in_pages =>
            {
                if let Some(x) = attr(&attrs, "fld").and_then(|s| s.parse().ok()) {
                    page_idx.push(x);
                }
            }
            XmlEvent::Empty { name: n, attrs } | XmlEvent::Start { name: n, attrs }
                if n == "dataField" =>
            {
                let fld = attr(&attrs, "fld")
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                let agg = attr(&attrs, "subtotal")
                    .and_then(PivotAgg::parse)
                    .unwrap_or(PivotAgg::Sum);
                let show_as = attr(&attrs, "showDataAs")
                    .and_then(ShowAs::parse)
                    .unwrap_or(ShowAs::Normal);
                data.push((fld, agg, show_as));
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
        omacell_core::addr::RefKind::Range(rg) => {
            (rg.start.row, rg.start.col, rg.end.row, rg.end.col)
        }
        omacell_core::addr::RefKind::Cell(c) => (c.row, c.col, c.row, c.col),
    };
    let source_sheet = cache
        .and_then(|c| {
            wb.sheets()
                .find(|s| s.name.eq_ignore_ascii_case(&c.source_sheet))
                .map(|s| s.id)
        })
        .unwrap_or(dest_sheet);
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
    table.refresh_on_load = cache.is_some_and(|c| c.refresh_on_load);
    table.rows = row_idx
        .into_iter()
        .filter_map(|i| headers.get(i).cloned())
        .collect();
    table.cols = col_idx
        .into_iter()
        .filter_map(|i| headers.get(i).cloned())
        .collect();
    table.filters = page_idx
        .into_iter()
        .filter_map(|i| headers.get(i).cloned())
        .map(|n| (n, Vec::new()))
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
    }
    let source_present = wb.sheet(source_sheet).is_some()
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
        let _ = wb.refresh_pivot(id);
    } else if !source_present {
        if let Some(cache) = cache {
            let _ = wb.refresh_pivot_from_cache(id, &cache.headers, &cache.rows);
        }
    }
    Ok(())
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
    let mut unique: BTreeMap<String, CacheValue> = BTreeMap::new();
    for value in values {
        match value {
            CacheValue::Number(n) => {
                has_num = true;
                unique.insert(n.to_string(), CacheValue::Number(*n));
            }
            CacheValue::Text(t) if !t.is_empty() => {
                has_str = true;
                unique.insert(t.clone(), CacheValue::Text(t.clone()));
            }
            _ => {}
        }
    }
    let mut xml = String::new();
    for v in unique.values() {
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
    if has_num && !has_str {
        attrs.push_str(r#" containsInteger="1""#);
    }
    (xml, attrs)
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
    Ok(format!(
        "{}{}:{}{}",
        col_to_letters(c0).map_err(|e| error::xlsx_write(e.to_string()))?,
        r0 + 1,
        col_to_letters(c1).map_err(|e| error::xlsx_write(e.to_string()))?,
        r1 + 1
    ))
}
