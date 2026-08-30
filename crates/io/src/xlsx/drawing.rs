//! DrawingML chart parts and sparkline XML from the core model.

use omacell_core::addr::col_to_letters;
use omacell_core::chart::{Chart, ChartKind, SparklineKind};
use omacell_core::error::CoreError;
use omacell_core::sheet::Sheet;

use super::xml::{XmlEvent, XmlReader};

const NS_XDR: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL_CHART: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart";
pub(crate) const REL_DRAWING: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing";
pub(crate) const CT_DRAWING: &str = "application/vnd.openxmlformats-officedocument.drawing+xml";
pub(crate) const CT_CHART: &str =
    "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";

/// Extra OPC parts + the worksheet `<drawing>` rel id.
pub(crate) struct ChartParts {
    pub drawing_rid: String,
    pub parts: Vec<(String, Vec<u8>, String)>,
    pub rels: Vec<(String, String, String, bool)>,
}

/// Emit drawing + chart parts for modeled charts on `sheet`.
pub(crate) fn chart_parts(
    sheet: &Sheet,
    sheet_ord: usize,
) -> Result<Option<ChartParts>, CoreError> {
    if sheet.charts.is_empty() {
        return Ok(None);
    }
    let drawing_name = format!("xl/drawings/drawing{}.xml", sheet_ord + 1);
    let mut drawing = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><xdr:wsDr xmlns:xdr="{NS_XDR}" xmlns:a="{NS_A}">"#
    );
    let mut drawing_rels = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    let mut parts = Vec::new();
    for (i, chart) in sheet.charts.iter().enumerate() {
        let crid = format!("rId{}", i + 1);
        let cname = format!("xl/charts/chart{}_{}.xml", sheet_ord + 1, i + 1);
        drawing.push_str(&anchor_xml(chart, &crid, i));
        drawing_rels.push_str(&format!(
            r#"<Relationship Id="{crid}" Type="{REL_CHART}" Target="../charts/chart{}_{}.xml"/>"#,
            sheet_ord + 1,
            i + 1
        ));
        parts.push((cname, chart_xml(sheet, chart)?, CT_CHART.into()));
    }
    drawing.push_str("</xdr:wsDr>");
    drawing_rels.push_str("</Relationships>");
    parts.push((
        drawing_name.clone(),
        drawing.into_bytes(),
        CT_DRAWING.into(),
    ));
    parts.push((
        format!("xl/drawings/_rels/drawing{}.xml.rels", sheet_ord + 1),
        drawing_rels.into_bytes(),
        String::new(),
    ));
    let drawing_rid = "rIdChart".to_string();
    let rels = vec![(
        drawing_rid.clone(),
        REL_DRAWING.into(),
        format!("../drawings/drawing{}.xml", sheet_ord + 1),
        false,
    )];
    Ok(Some(ChartParts {
        drawing_rid,
        parts,
        rels,
    }))
}

fn anchor_xml(chart: &Chart, rid: &str, index: usize) -> String {
    let a = chart.anchor;
    format!(
        r#"<xdr:twoCellAnchor><xdr:from><xdr:col>{}</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>{}</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:graphicFrame><xdr:nvGraphicFramePr><xdr:cNvPr id="{}" name="Chart {}"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm><a:graphic><a:graphicData uri="{NS_C}"><c:chart xmlns:c="{NS_C}" xmlns:r="{NS_R}" r:id="{rid}"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor>"#,
        a.from_col,
        a.from_row,
        a.to_col,
        a.to_row,
        index + 2,
        index + 1
    )
}

fn chart_xml(sheet: &Sheet, chart: &Chart) -> Result<Vec<u8>, CoreError> {
    let grouping = match chart.kind {
        ChartKind::ColumnStacked | ChartKind::BarStacked => "stacked",
        ChartKind::ColumnPct | ChartKind::BarPct => "percentStacked",
        _ => "clustered",
    };
    let bar_dir = if chart.kind.horizontal() {
        "bar"
    } else {
        "col"
    };
    let mut series = String::new();
    for (i, s) in chart.series.iter().enumerate() {
        let name = xml_escape(&s.name);
        let vals = range_a1(sheet, s.values);
        let cats = chart
            .categories
            .map(|r| range_a1(sheet, r))
            .unwrap_or_default();
        series.push_str(&format!(
            r#"<c:ser><c:idx val="{i}"/><c:order val="{i}"/><c:tx><c:v>{name}</c:v></c:tx>"#
        ));
        if !cats.is_empty() {
            series.push_str(&format!(
                r#"<c:cat><c:strRef><c:f>{}</c:f></c:strRef></c:cat>"#,
                xml_escape(&cats)
            ));
        }
        series.push_str(&format!(
            r#"<c:val><c:numRef><c:f>{}</c:f></c:numRef></c:val></c:ser>"#,
            xml_escape(&vals)
        ));
    }
    let plot = match chart.kind {
        ChartKind::Line | ChartKind::Combo => format!(
            r#"<c:lineChart><c:grouping val="standard"/>{series}<c:axId val="1"/><c:axId val="2"/></c:lineChart>"#
        ),
        ChartKind::Area => format!(
            r#"<c:areaChart><c:grouping val="stacked"/>{series}<c:axId val="1"/><c:axId val="2"/></c:areaChart>"#
        ),
        ChartKind::Pie | ChartKind::Donut => {
            let tag = if chart.kind == ChartKind::Donut {
                "doughnutChart"
            } else {
                "pieChart"
            };
            format!(r#"<c:{tag}>{series}</c:{tag}>"#)
        }
        ChartKind::Scatter | ChartKind::Bubble => {
            let tag = if chart.kind == ChartKind::Bubble {
                "bubbleChart"
            } else {
                "scatterChart"
            };
            format!(
                r#"<c:{tag}><c:scatterStyle val="lineMarker"/>{series}<c:axId val="1"/><c:axId val="2"/></c:{tag}>"#
            )
        }
        _ => format!(
            r#"<c:barChart><c:barDir val="{bar_dir}"/><c:grouping val="{grouping}"/>{series}<c:axId val="1"/><c:axId val="2"/></c:barChart>"#
        ),
    };
    let title = chart
        .title
        .as_deref()
        .map(|t| format!(r#"<c:title><c:tx><c:rich><a:p><a:r><a:t>{}</a:t></a:r></a:p></c:rich></c:tx></c:title>"#, xml_escape(t)))
        .unwrap_or_default();
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><c:chartSpace xmlns:c="{NS_C}" xmlns:a="{NS_A}" xmlns:r="{NS_R}"><c:chart>{title}<c:plotArea>{plot}<c:catAx><c:axId val="1"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:axPos val="b"/><c:crossAx val="2"/></c:catAx><c:valAx><c:axId val="2"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:axPos val="l"/><c:crossAx val="1"/></c:valAx></c:plotArea><c:legend><c:legendPos val="r"/></c:legend></c:chart></c:chartSpace>"#
    );
    Ok(xml.into_bytes())
}

fn range_a1(sheet: &Sheet, range: omacell_core::addr::RangeRef) -> String {
    let c0 = col_to_letters(range.start.col.min(range.end.col)).unwrap_or_else(|_| "A".into());
    let c1 = col_to_letters(range.start.col.max(range.end.col)).unwrap_or_else(|_| "A".into());
    let r0 = range.start.row.min(range.end.row) + 1;
    let r1 = range.start.row.max(range.end.row) + 1;
    format!("'{}'!{c0}{r0}:{c1}{r1}", sheet.name.replace('\'', "''"))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Sparkline extension fragment from the model.
pub(crate) fn sparkline_xml(sheet: &Sheet) -> Option<Vec<u8>> {
    if sheet.sparklines.is_empty() {
        return None;
    }
    let mut body = String::new();
    for sp in &sheet.sparklines {
        let ty = match sp.kind {
            SparklineKind::Line => "line",
            SparklineKind::Column => "column",
            SparklineKind::WinLoss => "stacked",
        };
        let data = range_a1(sheet, sp.data);
        let cell = format!(
            "{}{}",
            col_to_letters(sp.col).unwrap_or_else(|_| "A".into()),
            sp.row + 1
        );
        body.push_str(&format!(
            r#"<x14:sparklineGroup type="{ty}" displayEmptyCellsAs="gap" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:sparklines><x14:sparkline><xm:f>{}</xm:f><xm:sqref>{cell}</xm:sqref></x14:sparkline></x14:sparklines></x14:sparklineGroup>"#,
            xml_escape(&data)
        ));
    }
    Some(format!(
        r#"<x14:sparklineGroups xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main">{body}</x14:sparklineGroups>"#
    ).into_bytes())
}

/// Parse a generated chart part into a [`Chart`].
pub(crate) fn parse_chart_part(bytes: &[u8], sheet: omacell_core::addr::SheetId) -> Option<Chart> {
    let mut reader = XmlReader::new(bytes);
    let mut kind = ChartKind::Column;
    let mut title = None;
    let mut series: Vec<(String, String)> = Vec::new();
    let mut cur_name = String::new();
    let mut in_title = false;
    let mut in_f = false;
    while let Ok(Some(ev)) = reader.next() {
        match ev {
            XmlEvent::Start { name, .. } | XmlEvent::Empty { name, .. } => match name.as_str() {
                "lineChart" => kind = ChartKind::Line,
                "barChart" => kind = ChartKind::Bar,
                "areaChart" => kind = ChartKind::Area,
                "pieChart" => kind = ChartKind::Pie,
                "doughnutChart" => kind = ChartKind::Donut,
                "scatterChart" => kind = ChartKind::Scatter,
                "bubbleChart" => kind = ChartKind::Bubble,
                "title" => in_title = true,
                "f" => in_f = true,
                _ => {}
            },
            XmlEvent::End { name } => {
                if name == "title" {
                    in_title = false;
                }
                if name == "f" {
                    in_f = false;
                }
            }
            XmlEvent::Text(t) => {
                if in_title && title.is_none() {
                    title = Some(t);
                } else if in_f {
                    if cur_name.is_empty() {
                        cur_name = t;
                    } else {
                        series.push((std::mem::take(&mut cur_name), t));
                    }
                }
            }
        }
    }
    if series.is_empty() && title.is_none() {
        return None;
    }
    let parsed_series = series
        .into_iter()
        .filter_map(|(_name, formula)| {
            let range = omacell_core::addr::parse_a1(&formula).ok()?;
            match range.kind {
                omacell_core::addr::RefKind::Range(r) => Some(omacell_core::chart::Series {
                    name: _name,
                    values: r,
                    x: None,
                    size: None,
                    color: None,
                    secondary_axis: false,
                    trendline: None,
                }),
                omacell_core::addr::RefKind::Cell(c) => Some(omacell_core::chart::Series {
                    name: _name,
                    values: omacell_core::addr::RangeRef::from_corners(c, c),
                    x: None,
                    size: None,
                    color: None,
                    secondary_axis: false,
                    trendline: None,
                }),
            }
        })
        .collect::<Vec<_>>();
    Some(Chart {
        id: omacell_core::chart::ChartId::new(0),
        kind,
        title,
        categories: None,
        series: parsed_series,
        category_axis: omacell_core::chart::Axis::default(),
        value_axis: omacell_core::chart::Axis::default(),
        secondary_axis: None,
        legend: omacell_core::chart::LegendPos::Right,
        data_labels: false,
        anchor: omacell_core::chart::ChartAnchor::default(),
        sheet,
    })
}
