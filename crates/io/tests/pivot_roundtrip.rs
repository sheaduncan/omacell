//! Pivot `.xlsx` round-trip and external-loader structure checks.

use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::pivot::{PivotAgg, PivotCalcField, PivotDataField, PivotTable};
use omacell_core::workbook::{CalcMode, Workbook};
use omacell_io::xlsx::{open_bytes, save_bytes, save_workbook_bytes};
use std::io::{Cursor, Read, Write};
use zip::{ZipWriter, write::SimpleFileOptions};

fn seed() -> Workbook {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_text(s, 0, 0, "Region").unwrap();
    wb.set_text(s, 0, 1, "Amount").unwrap();
    wb.set_text(s, 0, 2, "Channel").unwrap();
    wb.set_text(s, 1, 0, "East").unwrap();
    wb.set_number(s, 1, 1, 10.0).unwrap();
    wb.set_text(s, 1, 2, "Online").unwrap();
    wb.set_text(s, 2, 0, "West").unwrap();
    wb.set_number(s, 2, 1, 70.0).unwrap();
    wb.set_text(s, 2, 2, "Store").unwrap();
    let mut table = PivotTable::new(
        "Sales",
        s,
        RangeRef::from_corners(CellRef::new(0, 0).unwrap(), CellRef::new(2, 2).unwrap()),
        s,
        0,
        4,
    );
    table.rows = vec!["Region".into()];
    table.data = vec![PivotDataField::new("Amount", PivotAgg::Sum)];
    table.filters = vec![("Channel".into(), vec!["Online".into()])];
    table.subtotals = false;
    table.refresh_on_load = true;
    wb.add_pivot(table).unwrap();
    wb
}

fn zip_names(bytes: &[u8]) -> Vec<String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect();
    names.sort();
    names
}

#[test]
fn modeled_pivot_writes_cache_and_table_parts() {
    let bytes = save_workbook_bytes(&seed()).unwrap();
    let names = zip_names(&bytes);
    assert!(
        names.iter().any(|n| n.contains("pivotCacheDefinition")),
        "{names:?}"
    );
    assert!(
        names.iter().any(|n| n.contains("pivotCacheRecords")),
        "{names:?}"
    );
    assert!(names.iter().any(|n| n.contains("pivotTable")), "{names:?}");
}

#[test]
fn modeled_pivot_round_trips() {
    let bytes = save_workbook_bytes(&seed()).unwrap();
    let doc = open_bytes(&bytes).unwrap();
    assert_eq!(doc.workbook.pivots().len(), 1);
    let pivot = doc.workbook.pivots().iter().next().unwrap();
    assert_eq!(pivot.name, "Sales");
    assert_eq!(pivot.rows, vec!["Region".to_string()]);
    assert_eq!(pivot.data[0].source, "Amount");
    assert_eq!(pivot.data[0].agg, PivotAgg::Sum);
    assert_eq!(
        pivot.filters,
        vec![("Channel".to_string(), vec!["Online".to_string()])]
    );
    assert!(!pivot.subtotals);
    let again = save_workbook_bytes(&doc.workbook).unwrap();
    let doc2 = open_bytes(&again).unwrap();
    assert_eq!(doc2.workbook.pivots().len(), 1);
}

#[test]
fn pivot_caches_follow_calc_properties_in_workbook_xml() {
    let mut wb = seed();
    wb.set_calc_mode(CalcMode::Manual).unwrap();
    let bytes = save_workbook_bytes(&wb).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    zip.by_name("xl/workbook.xml")
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    assert!(xml.find("<calcPr").unwrap() < xml.find("<pivotCaches").unwrap());
}

#[test]
fn openpyxl_sees_pivot_parts_if_present() {
    let bytes = save_workbook_bytes(&seed()).unwrap();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/test-tmp")
        .join(format!("omacell-pivot-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("pivot.xlsx");
    std::fs::write(&path, &bytes).unwrap();
    let output = std::process::Command::new("python3")
        .args([
            "-c",
            &format!(
                r#"
import zipfile, sys
z = zipfile.ZipFile(r'{path}')
names = z.namelist()
assert any('pivotCacheDefinition' in n for n in names), names
assert any('pivotTable' in n for n in names), names
try:
    import openpyxl
    wb = openpyxl.load_workbook(r'{path}')
    ws = wb.active
    pivots = getattr(ws, '_pivots', None) or []
    assert len(pivots) == 1, pivots
    print('openpyxl pivots', len(pivots))
except ImportError:
    print('openpyxl missing')
"#,
                path = path.display()
            ),
        ])
        .output();
    match output {
        Ok(out) if out.status.success() => {}
        Ok(out) => panic!(
            "openpyxl/zip check failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(_) => {}
    }
}

#[test]
fn libreoffice_opens_modeled_pivot_if_present() {
    let Some(soffice) = ["soffice", "libreoffice"]
        .into_iter()
        .find(|bin| which(bin))
    else {
        return;
    };
    let bytes = save_workbook_bytes(&seed()).unwrap();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/test-tmp")
        .join(format!("omacell-pivot-lo-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let dir = dir.canonicalize().unwrap();
    let path = dir.join("pivot.xlsx");
    let profile = dir.join("libreoffice-profile");
    std::fs::write(&path, bytes).unwrap();
    let out = std::process::Command::new(soffice)
        .arg(format!(
            "-env:UserInstallation=file://{}",
            profile.display()
        ))
        .env("HOME", &dir)
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("SAL_USE_VCLPLUGIN", "svp")
        .args([
            "--headless",
            "--convert-to",
            "ods",
            "--outdir",
            dir.to_str().unwrap(),
            path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "libreoffice failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let ods = dir.join("pivot.ods");
    assert!(ods.is_file(), "LibreOffice did not write {ods:?}");
    let mut zip = zip::ZipArchive::new(std::fs::File::open(ods).unwrap()).unwrap();
    let mut content = String::new();
    zip.by_name("content.xml")
        .unwrap()
        .read_to_string(&mut content)
        .unwrap();
    assert!(
        content.contains("table:data-pilot-table") && content.contains("table:name=\"Sales\""),
        "LibreOffice opened the workbook but did not retain a live pivot"
    );
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .and_then(|paths| std::env::split_paths(&paths).find(|p| p.join(bin).is_file()))
        .is_some()
}

fn zip_part(zip: &[u8], name: &str) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip.to_vec())).unwrap();
    let mut file = archive.by_name(name).unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    bytes
}

fn zip_text(zip: &[u8], name: &str) -> String {
    String::from_utf8(zip_part(zip, name)).unwrap()
}

fn package(files: &[(&str, &str)]) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut z = ZipWriter::new(&mut buf);
        let opt = SimpleFileOptions::default();
        for (name, body) in files {
            z.start_file(*name, opt).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }
    buf.into_inner()
}

const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_PKG: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const NS_CT: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const OD: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const PRESERVE_TOKEN: &str = "omacell-preserve-token";

fn excel_authored_pivot(calc: bool, distinct: bool, extra_ext: bool) -> Vec<u8> {
    let cache_ext = if extra_ext {
        format!(
            r#"<extLst><ext uri="{{725AE2AE-9491-48be-BD36-2398A43DD21F}}"><x14:pivotCacheDefinition/></ext><ext uri="{{00000000-0000-0000-0000-omacell000001}}"><omacellToken>{PRESERVE_TOKEN}</omacellToken></ext></extLst>"#
        )
    } else if distinct {
        r#"<extLst><ext uri="{725AE2AE-9491-48be-BD36-2398A43DD21F}"><x14:pivotCacheDefinition/></ext></extLst>"#
            .into()
    } else {
        String::new()
    };
    let calc_field = if calc {
        r#"<cacheField name="Tax" numFmtId="0" databaseField="0" formula="'Amount'*0.1"><sharedItems count="2" containsString="0" containsNumber="1" containsSemiMixedTypes="0" containsInteger="0"><n v="1"/><n v="7"/></sharedItems></cacheField>"#
    } else {
        ""
    };
    let field_count = if calc { 3 } else { 2 };
    let data_field = if distinct {
        r#"<dataField name="Distinct count of Region" fld="0" subtotal="count"><extLst><ext uri="{E15A36E0-9728-4e99-A89B-3F7291B0FE68}"><x14:dataField pivotShowAs="distinctCount"/></ext></extLst></dataField>"#
    } else if calc {
        r#"<dataField name="Sum of Tax" fld="2" subtotal="sum"/>"#
    } else {
        r#"<dataField name="Sum of Amount" fld="1" subtotal="sum"/>"#
    };
    let x14_ns = if distinct || extra_ext {
        format!(r#" xmlns:x14="{NS_X14}""#)
    } else {
        String::new()
    };
    package(&[
        (
            "[Content_Types].xml",
            &format!(
                r#"<?xml version="1.0"?><Types xmlns="{NS_CT}"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/pivotCache/pivotCacheDefinition1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml"/><Override PartName="/xl/pivotCache/pivotCacheRecords1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheRecords+xml"/><Override PartName="/xl/pivotTables/pivotTable1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml"/></Types>"#
            ),
        ),
        (
            "_rels/.rels",
            &format!(
                r#"<?xml version="1.0"?><Relationships xmlns="{NS_PKG}"><Relationship Id="rId1" Type="{OD}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            &format!(
                r#"<?xml version="1.0"?><Relationships xmlns="{NS_PKG}"><Relationship Id="rId1" Type="{OD}/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="{OD}/pivotCacheDefinition" Target="pivotCache/pivotCacheDefinition1.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/workbook.xml",
            &format!(
                r#"<?xml version="1.0"?><workbook xmlns="{NS}" xmlns:r="{NS_R}"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets><pivotCaches count="1"><pivotCache cacheId="2" r:id="rId2"/></pivotCaches></workbook>"#
            ),
        ),
        (
            "xl/worksheets/_rels/sheet1.xml.rels",
            &format!(
                r#"<?xml version="1.0"?><Relationships xmlns="{NS_PKG}"><Relationship Id="rId1" Type="{OD}/pivotTable" Target="../pivotTables/pivotTable1.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/worksheets/sheet1.xml",
            &format!(
                r#"<?xml version="1.0"?><worksheet xmlns="{NS}" xmlns:r="{NS_R}"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Region</t></is></c><c r="B1" t="inlineStr"><is><t>Amount</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>East</t></is></c><c r="B2"><v>10</v></c></row><row r="3"><c r="A3" t="inlineStr"><is><t>West</t></is></c><c r="B3"><v>70</v></c></row></sheetData></worksheet>"#
            ),
        ),
        (
            "xl/pivotCache/_rels/pivotCacheDefinition1.xml.rels",
            &format!(
                r#"<?xml version="1.0"?><Relationships xmlns="{NS_PKG}"><Relationship Id="rId1" Type="{OD}/pivotCacheRecords" Target="pivotCacheRecords1.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/pivotCache/pivotCacheDefinition1.xml",
            &format!(
                r#"<?xml version="1.0"?><pivotCacheDefinition xmlns="{NS}" xmlns:r="{NS_R}"{x14_ns} r:id="rId1" refreshOnLoad="0" recordCount="2" createdVersion="8" refreshedVersion="8"><cacheSource type="worksheet"><worksheetSource ref="A1:B3" sheet="Sheet1"/></cacheSource><cacheFields count="{field_count}"><cacheField name="Region" numFmtId="0"><sharedItems count="2" containsString="1" containsNumber="0" containsSemiMixedTypes="0"><s v="East"/><s v="West"/></sharedItems></cacheField><cacheField name="Amount" numFmtId="0"><sharedItems count="2" containsString="0" containsNumber="1" containsSemiMixedTypes="0" containsInteger="1"><n v="10"/><n v="70"/></sharedItems></cacheField>{calc_field}</cacheFields>{cache_ext}</pivotCacheDefinition>"#
            ),
        ),
        (
            "xl/pivotCache/pivotCacheRecords1.xml",
            &format!(
                r#"<?xml version="1.0"?><pivotCacheRecords xmlns="{NS}" count="2"><r><s v="East"/><n v="10"/>{}</r><r><s v="West"/><n v="70"/>{}</r></pivotCacheRecords>"#,
                if calc { "<n v=\"1\"/>" } else { "" },
                if calc { "<n v=\"7\"/>" } else { "" },
            ),
        ),
        (
            "xl/pivotTables/_rels/pivotTable1.xml.rels",
            &format!(
                r#"<?xml version="1.0"?><Relationships xmlns="{NS_PKG}"><Relationship Id="rId1" Type="{OD}/pivotCacheDefinition" Target="../pivotCache/pivotCacheDefinition1.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/pivotTables/pivotTable1.xml",
            &format!(
                r#"<?xml version="1.0"?><pivotTableDefinition xmlns="{NS}"{x14_ns} name="Sales" cacheId="2" dataCaption="Values"><location ref="E1:F4" firstHeaderRow="1" firstDataRow="1" firstDataCol="1"/><pivotFields count="{field_count}"><pivotField axis="axisRow" showAll="0"><items count="3"><item x="0"/><item x="1"/><item t="default"/></items></pivotField><pivotField showAll="0"/>{calc_axis}</pivotFields><rowFields count="1"><field x="0"/></rowFields><dataFields count="1">{data_field}</dataFields></pivotTableDefinition>"#,
                calc_axis = if calc {
                    r#"<pivotField dataField="1" showAll="0"/>"#
                } else {
                    ""
                },
            ),
        ),
    ])
}

#[test]
fn unchanged_pivot_preserves_opaque_extension_bytes() {
    let original = excel_authored_pivot(false, false, true);
    let original_cache = zip_part(&original, "xl/pivotCache/pivotCacheDefinition1.xml");
    assert!(
        String::from_utf8_lossy(&original_cache).contains(PRESERVE_TOKEN),
        "fixture must carry the opaque extension"
    );
    let doc = open_bytes(&original).unwrap();
    assert_eq!(doc.workbook.pivots().len(), 1);
    assert!(!doc.workbook.pivots().iter().next().unwrap().ooxml_dirty);
    let saved = save_bytes(&doc).unwrap();
    let saved_cache = zip_part(&saved, "xl/pivotCache/pivotCacheDefinition1.xml");
    assert_eq!(saved_cache, original_cache);
    let saved_table = zip_part(&saved, "xl/pivotTables/pivotTable1.xml");
    let original_table = zip_part(&original, "xl/pivotTables/pivotTable1.xml");
    assert_eq!(saved_table, original_table);
}

#[test]
fn adding_a_pivot_does_not_reuse_an_imported_cache_id_or_part_name() {
    let original = excel_authored_pivot(false, false, false);
    let mut doc = open_bytes(&original).unwrap();
    let sheet = doc.workbook.active_sheet();
    let mut added = PivotTable::new(
        "Added",
        sheet,
        RangeRef::from_corners(CellRef::new(0, 0).unwrap(), CellRef::new(2, 1).unwrap()),
        sheet,
        10,
        0,
    );
    added.rows = vec!["Region".into()];
    added.data = vec![PivotDataField::new("Amount", PivotAgg::Sum)];
    doc.workbook.add_pivot(added).unwrap();

    let saved = save_bytes(&doc).unwrap();
    let cache_defs: Vec<_> = zip_names(&saved)
        .into_iter()
        .filter(|name| name.contains("pivotCacheDefinition") && name.ends_with(".xml"))
        .collect();
    assert_eq!(cache_defs.len(), 2, "{cache_defs:?}");
    let reopened = open_bytes(&saved).unwrap();
    assert_eq!(reopened.workbook.pivots().len(), 2);
}

#[test]
fn refresh_regenerates_dirty_pivot_parts() {
    let original = excel_authored_pivot(false, false, true);
    let mut doc = open_bytes(&original).unwrap();
    let id = doc.workbook.pivots().iter().next().unwrap().id;
    doc.workbook.refresh_pivot(id).unwrap();
    assert!(doc.workbook.pivots().get(id).unwrap().ooxml_dirty);
    let saved = save_bytes(&doc).unwrap();
    let cache = zip_text(&saved, "xl/pivotCache/pivotCacheDefinition1.xml");
    assert!(
        !cache.contains(PRESERVE_TOKEN),
        "dirty pivots must regenerate rather than copy unsupported extensions"
    );
}

#[test]
fn calculated_field_fixture_round_trips_without_downgrade() {
    let bytes = excel_authored_pivot(true, false, false);
    let doc = open_bytes(&bytes).unwrap();
    let pivot = doc.workbook.pivots().iter().next().unwrap();
    assert_eq!(pivot.calc_fields.len(), 1);
    assert_eq!(pivot.calc_fields[0].name, "Tax");
    assert_eq!(pivot.calc_fields[0].formula, "'Amount'*0.1");
    assert_eq!(pivot.data[0].source, "Tax");
    let saved = save_bytes(&doc).unwrap();
    let cache = zip_text(&saved, "xl/pivotCache/pivotCacheDefinition1.xml");
    assert!(cache.contains(r#"databaseField="0""#), "{cache}");
    assert!(cache.contains("formula="), "{cache}");
}

#[test]
fn distinct_count_fixture_round_trips_x14_metadata() {
    let bytes = excel_authored_pivot(false, true, false);
    let doc = open_bytes(&bytes).unwrap();
    let pivot = doc.workbook.pivots().iter().next().unwrap();
    assert_eq!(pivot.data[0].agg, PivotAgg::DistinctCount);
    let saved = save_bytes(&doc).unwrap();
    let table = zip_text(&saved, "xl/pivotTables/pivotTable1.xml");
    assert!(table.contains("distinctCount"), "{table}");
}

#[test]
fn generated_distinct_count_writes_x14_and_calc_field_formula() {
    let mut wb = seed();
    let s = wb.active_sheet();
    let mut distinct = PivotTable::new(
        "Distinct",
        s,
        RangeRef::from_corners(CellRef::new(0, 0).unwrap(), CellRef::new(2, 2).unwrap()),
        s,
        20,
        0,
    );
    distinct.rows = vec!["Channel".into()];
    distinct.data = vec![PivotDataField::new("Region", PivotAgg::DistinctCount)];
    wb.add_pivot(distinct).unwrap();
    let mut tax = PivotTable::new(
        "Tax",
        s,
        RangeRef::from_corners(CellRef::new(0, 0).unwrap(), CellRef::new(2, 1).unwrap()),
        s,
        30,
        0,
    );
    tax.rows = vec!["Region".into()];
    tax.calc_fields = vec![PivotCalcField {
        name: "Tax".into(),
        formula: "'Amount'*0.1".into(),
    }];
    tax.data = vec![PivotDataField::new("Tax", PivotAgg::Sum)];
    wb.add_pivot(tax).unwrap();
    let bytes = save_workbook_bytes(&wb).unwrap();
    let names = zip_names(&bytes);
    let table_xml: String = names
        .iter()
        .filter(|n| n.contains("pivotTable"))
        .map(|n| zip_text(&bytes, n))
        .collect();
    assert!(table_xml.contains("distinctCount"), "{table_xml}");
    let cache_xml: String = names
        .iter()
        .filter(|n| n.contains("pivotCacheDefinition"))
        .map(|n| zip_text(&bytes, n))
        .collect();
    assert!(cache_xml.contains(r#"databaseField="0""#), "{cache_xml}");
    let doc = open_bytes(&bytes).unwrap();
    let distinct = doc
        .workbook
        .pivots()
        .iter()
        .find(|pivot| pivot.name == "Distinct")
        .unwrap();
    assert_eq!(distinct.data[0].agg, PivotAgg::DistinctCount);
    let tax = doc
        .workbook
        .pivots()
        .iter()
        .find(|pivot| pivot.name == "Tax")
        .unwrap();
    assert_eq!(tax.calc_fields[0].name, "Tax");
}

#[test]
fn libreoffice_opens_calc_and_distinct_fixtures_if_present() {
    let Some(soffice) = ["soffice", "libreoffice"]
        .into_iter()
        .find(|bin| which(bin))
    else {
        return;
    };
    for (label, bytes) in [
        ("calc", excel_authored_pivot(true, false, false)),
        ("distinct", excel_authored_pivot(false, true, false)),
    ] {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-tmp")
            .join(format!("omacell-pivot-24a-{label}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let dir = dir.canonicalize().unwrap();
        let path = dir.join("pivot.xlsx");
        let profile = dir.join("libreoffice-profile");
        std::fs::write(&path, bytes).unwrap();
        let out = std::process::Command::new(soffice)
            .arg(format!(
                "-env:UserInstallation=file://{}",
                profile.display()
            ))
            .env("HOME", &dir)
            .env("XDG_CACHE_HOME", dir.join("cache"))
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .env("SAL_USE_VCLPLUGIN", "svp")
            .args([
                "--headless",
                "--convert-to",
                "ods",
                "--outdir",
                dir.to_str().unwrap(),
                path.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{label} libreoffice failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let ods = dir.join("pivot.ods");
        assert!(ods.is_file(), "{label}: LibreOffice did not write {ods:?}");
        let mut zip = zip::ZipArchive::new(std::fs::File::open(ods).unwrap()).unwrap();
        let mut content = String::new();
        zip.by_name("content.xml")
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();
        assert!(
            content.contains("table:data-pilot-table"),
            "{label}: LibreOffice dropped the live pivot"
        );
    }
}
