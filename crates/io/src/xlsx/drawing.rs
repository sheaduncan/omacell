//! DrawingML chart parts and sparkline XML from the core model.

use omacell_core::addr::col_to_letters;
use std::collections::HashMap;

use omacell_core::chart::{
    Axis, Chart, ChartAnchor, ChartId, ChartKind, LegendPos, Series, Sparkline, SparklineKind,
    Trendline, TrendlineKind,
};
use omacell_core::error::CoreError;
use omacell_core::sheet::Sheet;
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;

use super::xml::{XmlEvent, XmlReader, attr};

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
    wb: &Workbook,
    sheet: &Sheet,
    sheet_ord: usize,
) -> Result<Option<ChartParts>, CoreError> {
    if sheet.charts.is_empty() {
        return Ok(None);
    }
    if sheet
        .charts
        .iter()
        .any(|chart| chart.kind == ChartKind::Unsupported)
    {
        // An unsupported source chart is represented as a GUI placeholder, but
        // its DrawingML remains authoritative and must stay byte-identical.
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
        parts.push((cname, chart_xml(wb, sheet, chart)?, CT_CHART.into()));
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

fn chart_xml(wb: &Workbook, sheet: &Sheet, chart: &Chart) -> Result<Vec<u8>, CoreError> {
    chart.values_valid()?;
    let indexed = chart.series.iter().enumerate().collect::<Vec<_>>();
    let labels = data_labels_xml(chart);
    let plot = match chart.kind {
        ChartKind::Line => {
            let series = series_xml(wb, sheet, chart, &indexed, SeriesXmlKind::Category);
            format!(
                r#"<c:lineChart><c:grouping val="standard"/>{series}{labels}<c:axId val="1"/><c:axId val="2"/></c:lineChart>"#
            )
        }
        ChartKind::Combo => {
            let mut primary = indexed
                .iter()
                .copied()
                .filter(|(_, series)| !series.secondary_axis)
                .collect::<Vec<_>>();
            let mut secondary = indexed
                .iter()
                .copied()
                .filter(|(_, series)| series.secondary_axis)
                .collect::<Vec<_>>();
            if primary.is_empty() && !secondary.is_empty() {
                primary.push(secondary.remove(0));
            }
            let bars = series_xml(wb, sheet, chart, &primary, SeriesXmlKind::Category);
            let lines = series_xml(wb, sheet, chart, &secondary, SeriesXmlKind::Category);
            format!(
                r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{bars}{labels}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:lineChart><c:grouping val="standard"/>{lines}{labels}<c:axId val="1"/><c:axId val="3"/></c:lineChart>"#
            )
        }
        ChartKind::Area => {
            let series = series_xml(wb, sheet, chart, &indexed, SeriesXmlKind::Category);
            format!(
                r#"<c:areaChart><c:grouping val="stacked"/>{series}{labels}<c:axId val="1"/><c:axId val="2"/></c:areaChart>"#
            )
        }
        ChartKind::Pie | ChartKind::Donut => {
            let series = series_xml(wb, sheet, chart, &indexed, SeriesXmlKind::Category);
            let tag = if chart.kind == ChartKind::Donut {
                "doughnutChart"
            } else {
                "pieChart"
            };
            format!(r#"<c:{tag}>{series}{labels}</c:{tag}>"#)
        }
        ChartKind::Scatter => {
            let series = series_xml(wb, sheet, chart, &indexed, SeriesXmlKind::Scatter);
            format!(
                r#"<c:scatterChart><c:scatterStyle val="marker"/>{series}{labels}<c:axId val="1"/><c:axId val="2"/></c:scatterChart>"#
            )
        }
        ChartKind::Bubble => {
            let series = series_xml(wb, sheet, chart, &indexed, SeriesXmlKind::Bubble);
            format!(
                r#"<c:bubbleChart><c:varyColors val="0"/>{series}{labels}<c:axId val="1"/><c:axId val="2"/></c:bubbleChart>"#
            )
        }
        ChartKind::Unsupported => String::new(),
        kind => {
            let grouping = match kind {
                ChartKind::ColumnStacked | ChartKind::BarStacked => "stacked",
                ChartKind::ColumnPct | ChartKind::BarPct => "percentStacked",
                _ => "clustered",
            };
            let bar_dir = if kind.horizontal() { "bar" } else { "col" };
            let series = series_xml(wb, sheet, chart, &indexed, SeriesXmlKind::Category);
            let marker = if kind == ChartKind::Histogram {
                r#"<c:extLst><c:ext uri="{D629F2F2-AC7D-4B6D-A53F-4433B4C80D54}"><om:kind xmlns:om="https://omacell.dev/chart/1" val="histogram"/></c:ext></c:extLst>"#
            } else {
                ""
            };
            format!(
                r#"<c:barChart><c:barDir val="{bar_dir}"/><c:grouping val="{grouping}"/>{series}{labels}<c:axId val="1"/><c:axId val="2"/>{marker}</c:barChart>"#
            )
        }
    };
    let title = chart
        .title
        .as_deref()
        .map(|t| format!(r#"<c:title><c:tx><c:rich><a:p><a:r><a:t>{}</a:t></a:r></a:p></c:rich></c:tx></c:title>"#, xml_escape(t)))
        .unwrap_or_default();
    let axes = axes_xml(chart);
    let legend = match chart.legend {
        LegendPos::None => String::new(),
        LegendPos::Right => r#"<c:legend><c:legendPos val="r"/></c:legend>"#.into(),
        LegendPos::Bottom => r#"<c:legend><c:legendPos val="b"/></c:legend>"#.into(),
    };
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><c:chartSpace xmlns:c="{NS_C}" xmlns:a="{NS_A}" xmlns:r="{NS_R}"><c:chart>{title}<c:plotArea>{plot}{axes}</c:plotArea>{legend}<c:plotVisOnly val="1"/></c:chart></c:chartSpace>"#
    );
    Ok(xml.into_bytes())
}

#[derive(Clone, Copy)]
enum SeriesXmlKind {
    Category,
    Scatter,
    Bubble,
}

fn series_xml(
    wb: &Workbook,
    sheet: &Sheet,
    chart: &Chart,
    series: &[(usize, &Series)],
    kind: SeriesXmlKind,
) -> String {
    let mut out = String::new();
    for (index, item) in series {
        let name = xml_escape(&item.name);
        out.push_str(&format!(
            r#"<c:ser><c:idx val="{index}"/><c:order val="{index}"/><c:tx><c:v>{name}</c:v></c:tx>"#
        ));
        match kind {
            SeriesXmlKind::Category => {
                if let Some(categories) = chart.categories {
                    let range = range_a1(wb, sheet, categories);
                    out.push_str(&format!(
                        r#"<c:cat><c:strRef><c:f>{}</c:f></c:strRef></c:cat>"#,
                        xml_escape(&range)
                    ));
                }
                let values = range_a1(wb, sheet, item.values);
                out.push_str(&format!(
                    r#"<c:val><c:numRef><c:f>{}</c:f></c:numRef></c:val>"#,
                    xml_escape(&values)
                ));
            }
            SeriesXmlKind::Scatter | SeriesXmlKind::Bubble => {
                let x = item.x.unwrap_or(item.values);
                let x = range_a1(wb, sheet, x);
                let y = range_a1(wb, sheet, item.values);
                out.push_str(&format!(r#"<c:xVal><c:numRef><c:f>{}</c:f></c:numRef></c:xVal><c:yVal><c:numRef><c:f>{}</c:f></c:numRef></c:yVal>"#, xml_escape(&x), xml_escape(&y)));
                if matches!(kind, SeriesXmlKind::Bubble)
                    && let Some(size) = item.size
                {
                    let size = range_a1(wb, sheet, size);
                    out.push_str(&format!(
                        r#"<c:bubbleSize><c:numRef><c:f>{}</c:f></c:numRef></c:bubbleSize>"#,
                        xml_escape(&size)
                    ));
                }
            }
        }
        if let Some(color) = item.color.as_deref().and_then(srgb_color) {
            out.push_str(&format!(
                r#"<c:spPr><a:solidFill><a:srgbClr val="{color}"/></a:solidFill></c:spPr>"#
            ));
        }
        if let Some(trendline) = &item.trendline {
            let kind = match trendline.kind {
                TrendlineKind::Linear => "linear",
                TrendlineKind::Exponential => "exp",
                TrendlineKind::MovingAverage => "movingAvg",
            };
            out.push_str(&format!(r#"<c:trendline><c:trendlineType val="{kind}"/>"#));
            if trendline.kind == TrendlineKind::MovingAverage {
                out.push_str(&format!(r#"<c:period val="{}"/>"#, trendline.period.max(2)));
            }
            out.push_str("</c:trendline>");
        }
        out.push_str("</c:ser>");
    }
    out
}

fn data_labels_xml(chart: &Chart) -> &'static str {
    if chart.data_labels {
        r#"<c:dLbls><c:showVal val="1"/></c:dLbls>"#
    } else {
        ""
    }
}

fn axes_xml(chart: &Chart) -> String {
    if matches!(chart.kind, ChartKind::Pie | ChartKind::Donut) {
        return String::new();
    }
    let category = axis_xml("catAx", 1, "b", 2, &chart.category_axis);
    let primary = axis_xml("valAx", 2, "l", 1, &chart.value_axis);
    let secondary = if chart.kind == ChartKind::Combo {
        chart
            .secondary_axis
            .as_ref()
            .map(|axis| axis_xml("valAx", 3, "r", 1, axis))
            .unwrap_or_default()
    } else {
        String::new()
    };
    if matches!(chart.kind, ChartKind::Scatter | ChartKind::Bubble) {
        format!(
            "{}{}",
            axis_xml("valAx", 1, "b", 2, &chart.category_axis),
            primary
        )
    } else {
        format!("{category}{primary}{secondary}")
    }
}

fn axis_xml(tag: &str, id: u32, position: &str, cross: u32, axis: &Axis) -> String {
    let title = axis
        .title
        .as_deref()
        .map(axis_title_xml)
        .unwrap_or_default();
    let gridlines = if axis.gridlines && tag == "valAx" {
        "<c:majorGridlines/>"
    } else {
        ""
    };
    format!(
        r#"<c:{tag}><c:axId val="{id}"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:axPos val="{position}"/>{title}{gridlines}<c:crossAx val="{cross}"/></c:{tag}>"#
    )
}

fn axis_title_xml(title: &str) -> String {
    format!(
        r#"<c:title><c:tx><c:rich><a:p><a:r><a:t>{}</a:t></a:r></a:p></c:rich></c:tx></c:title>"#,
        xml_escape(title)
    )
}

fn srgb_color(color: &str) -> Option<&str> {
    let value = color.strip_prefix('#')?;
    (value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(value)
}

fn range_a1(wb: &Workbook, sheet: &Sheet, range: omacell_core::addr::RangeRef) -> String {
    let c0 = col_to_letters(range.start.col.min(range.end.col)).unwrap_or_else(|_| "A".into());
    let c1 = col_to_letters(range.start.col.max(range.end.col)).unwrap_or_else(|_| "A".into());
    let r0 = range.start.row.min(range.end.row) + 1;
    let r1 = range.start.row.max(range.end.row) + 1;
    let range_sheet = range
        .start
        .sheet
        .and_then(|id| wb.sheet(id))
        .unwrap_or(sheet);
    format!(
        "'{}'!{c0}{r0}:{c1}{r1}",
        range_sheet.name.replace('\'', "''")
    )
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if !is_xml10_char(ch) {
            out.push('\u{FFFD}');
            continue;
        }
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

fn is_xml10_char(ch: char) -> bool {
    matches!(ch, '\u{9}' | '\u{A}' | '\u{D}')
        || ('\u{20}'..='\u{D7FF}').contains(&ch)
        || ('\u{E000}'..='\u{FFFD}').contains(&ch)
        || ('\u{10000}'..='\u{10FFFF}').contains(&ch)
}

/// Sparkline extension fragment from the model.
pub(crate) fn sparkline_xml(wb: &Workbook, sheet: &Sheet) -> Option<Vec<u8>> {
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
        let data = range_a1(wb, sheet, sp.data);
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

/// Drawing relationship ids mapped to their two-cell anchors.
pub(crate) fn parse_drawing_anchors(bytes: &[u8]) -> HashMap<String, ChartAnchor> {
    #[derive(Clone, Copy)]
    enum Corner {
        From,
        To,
    }
    #[derive(Clone, Copy)]
    enum Coord {
        Row,
        Col,
    }
    #[derive(Default)]
    struct Pending {
        from_row: Option<u32>,
        from_col: Option<u16>,
        to_row: Option<u32>,
        to_col: Option<u16>,
        rid: Option<String>,
    }

    let mut reader = XmlReader::new(bytes);
    let mut out = HashMap::new();
    let mut pending: Option<Pending> = None;
    let mut corner = None;
    let mut coord = None;
    while let Ok(Some(event)) = reader.next() {
        match event {
            XmlEvent::Start { name, attrs } | XmlEvent::Empty { name, attrs } => {
                match name.as_str() {
                    "twoCellAnchor" => pending = Some(Pending::default()),
                    "from" => corner = Some(Corner::From),
                    "to" => corner = Some(Corner::To),
                    "row" => coord = Some(Coord::Row),
                    "col" => coord = Some(Coord::Col),
                    "chart" => {
                        if let Some(target) = pending.as_mut()
                            && let Some(id) = attr(&attrs, "id")
                        {
                            target.rid = Some(id.to_string());
                        }
                    }
                    _ => {}
                }
            }
            XmlEvent::Text(text) => {
                let Some(target) = pending.as_mut() else {
                    continue;
                };
                match (corner, coord) {
                    (Some(Corner::From), Some(Coord::Row)) => {
                        target.from_row = text.trim().parse().ok();
                    }
                    (Some(Corner::From), Some(Coord::Col)) => {
                        target.from_col = text.trim().parse().ok();
                    }
                    (Some(Corner::To), Some(Coord::Row)) => {
                        target.to_row = text.trim().parse().ok();
                    }
                    (Some(Corner::To), Some(Coord::Col)) => {
                        target.to_col = text.trim().parse().ok();
                    }
                    _ => {}
                }
            }
            XmlEvent::End { name } => match name.as_str() {
                "row" | "col" => coord = None,
                "from" | "to" => corner = None,
                "twoCellAnchor" => {
                    if let Some(target) = pending.take()
                        && let (
                            Some(rid),
                            Some(from_row),
                            Some(from_col),
                            Some(to_row),
                            Some(to_col),
                        ) = (
                            target.rid,
                            target.from_row,
                            target.from_col,
                            target.to_row,
                            target.to_col,
                        )
                        && from_row <= to_row
                        && from_col <= to_col
                        && omacell_core::addr::CellRef::new(from_row, from_col).is_ok()
                        && omacell_core::addr::CellRef::new(to_row, to_col).is_ok()
                    {
                        out.insert(
                            rid,
                            ChartAnchor {
                                from_row,
                                from_col,
                                to_row,
                                to_col,
                            },
                        );
                    }
                }
                _ => {}
            },
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChartGroup {
    Line,
    Bar,
    Area,
    Pie,
    Donut,
    Scatter,
    Bubble,
    Unknown,
}

#[derive(Clone, Copy)]
enum FormulaField {
    Categories,
    X,
    Y,
    Size,
}

struct RawSeries {
    group: ChartGroup,
    name: String,
    name_formula: String,
    categories: Option<String>,
    x: Option<String>,
    y: Option<String>,
    size: Option<String>,
    color: Option<String>,
    trendline: Option<Trendline>,
}

impl RawSeries {
    fn new(group: ChartGroup) -> Self {
        Self {
            group,
            name: String::new(),
            name_formula: String::new(),
            categories: None,
            x: None,
            y: None,
            size: None,
            color: None,
            trendline: None,
        }
    }

    fn set_formula(&mut self, field: FormulaField, formula: String) {
        let target = match field {
            FormulaField::Categories => &mut self.categories,
            FormulaField::X => &mut self.x,
            FormulaField::Y => &mut self.y,
            FormulaField::Size => &mut self.size,
        };
        target.get_or_insert_default().push_str(&formula);
    }
}

/// Parse a chart part into a modeled chart or an unsupported placeholder.
pub(crate) fn parse_chart_part(
    bytes: &[u8],
    wb: &Workbook,
    sheet: omacell_core::addr::SheetId,
    anchor: ChartAnchor,
) -> Option<Chart> {
    let mut reader = XmlReader::new(bytes);
    let mut groups = Vec::new();
    let mut group = None;
    let mut raw = Vec::new();
    let mut current: Option<RawSeries> = None;
    let mut field = None;
    let mut in_formula = false;
    let mut in_series_name = false;
    let mut in_series_value = false;
    let mut in_title = false;
    let mut in_title_text = false;
    let mut in_plot_area = false;
    let mut title = String::new();
    let mut bar_dir = "col".to_string();
    let mut grouping = "clustered".to_string();
    let mut histogram = false;
    let mut data_labels = false;
    let mut legend = LegendPos::None;
    let mut saw_legend = false;

    while let Ok(Some(event)) = reader.next() {
        match event {
            XmlEvent::Start { name, attrs } | XmlEvent::Empty { name, attrs } => {
                let next_group = match name.as_str() {
                    "lineChart" => Some(ChartGroup::Line),
                    "barChart" => Some(ChartGroup::Bar),
                    "areaChart" => Some(ChartGroup::Area),
                    "pieChart" => Some(ChartGroup::Pie),
                    "doughnutChart" => Some(ChartGroup::Donut),
                    "scatterChart" => Some(ChartGroup::Scatter),
                    "bubbleChart" => Some(ChartGroup::Bubble),
                    _ if name.ends_with("Chart") => Some(ChartGroup::Unknown),
                    _ => None,
                };
                if let Some(next) = next_group {
                    group = Some(next);
                    groups.push(next);
                }
                match name.as_str() {
                    "ser" => current = Some(RawSeries::new(group.unwrap_or(ChartGroup::Unknown))),
                    "tx" if current.is_some() => in_series_name = true,
                    "v" if in_series_name => in_series_value = true,
                    "cat" => field = Some(FormulaField::Categories),
                    "xVal" => field = Some(FormulaField::X),
                    "yVal" | "val" => field = Some(FormulaField::Y),
                    "bubbleSize" => field = Some(FormulaField::Size),
                    "f" if current.is_some() => in_formula = true,
                    "plotArea" => in_plot_area = true,
                    "title" if current.is_none() && !in_plot_area && title.is_empty() => {
                        in_title = true;
                    }
                    "t" | "v" if in_title => in_title_text = true,
                    "barDir" => {
                        if let Some(value) = attr(&attrs, "val") {
                            bar_dir = value.to_string();
                        }
                    }
                    "grouping" => {
                        if let Some(value) = attr(&attrs, "val") {
                            grouping = value.to_string();
                        }
                    }
                    "kind" if attr(&attrs, "val") == Some("histogram") => histogram = true,
                    "showVal" if attr(&attrs, "val").is_some_and(truthy) => data_labels = true,
                    "legend" => saw_legend = true,
                    "legendPos" => {
                        legend = match attr(&attrs, "val") {
                            Some("b") => LegendPos::Bottom,
                            _ => LegendPos::Right,
                        };
                    }
                    "srgbClr" => {
                        if let Some(series) = current.as_mut()
                            && let Some(value) = attr(&attrs, "val")
                            && value.len() == 6
                            && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                        {
                            series.color = Some(format!("#{value}"));
                        }
                    }
                    "trendline" => {
                        if let Some(series) = current.as_mut() {
                            series.trendline = Some(Trendline {
                                kind: TrendlineKind::Linear,
                                period: 2,
                            });
                        }
                    }
                    "trendlineType" => {
                        if let Some(trendline) = current
                            .as_mut()
                            .and_then(|series| series.trendline.as_mut())
                        {
                            trendline.kind = match attr(&attrs, "val") {
                                Some("exp") => TrendlineKind::Exponential,
                                Some("movingAvg") => TrendlineKind::MovingAverage,
                                _ => TrendlineKind::Linear,
                            };
                        }
                    }
                    "period" => {
                        if let Some(trendline) = current
                            .as_mut()
                            .and_then(|series| series.trendline.as_mut())
                            && let Some(period) =
                                attr(&attrs, "val").and_then(|value| value.parse().ok())
                        {
                            trendline.period = period;
                        }
                    }
                    _ => {}
                }
            }
            XmlEvent::Text(text) => {
                if in_title && in_title_text {
                    title.push_str(&text);
                } else if in_series_name && in_series_value {
                    if let Some(series) = current.as_mut() {
                        series.name.push_str(&text);
                    }
                } else if in_formula {
                    if in_series_name {
                        if let Some(series) = current.as_mut() {
                            series.name_formula.push_str(&text);
                        }
                    } else if let (Some(series), Some(target)) = (current.as_mut(), field) {
                        series.set_formula(target, text);
                    }
                }
            }
            XmlEvent::End { name } => match name.as_str() {
                "ser" => {
                    if let Some(series) = current.take() {
                        raw.push(series);
                    }
                }
                "tx" => in_series_name = false,
                "v" => {
                    in_series_value = false;
                    in_title_text = false;
                }
                "t" => in_title_text = false,
                "cat" | "xVal" | "yVal" | "val" | "bubbleSize" => field = None,
                "f" => in_formula = false,
                "title" => in_title = false,
                "plotArea" => in_plot_area = false,
                "lineChart" | "barChart" | "areaChart" | "pieChart" | "doughnutChart"
                | "scatterChart" | "bubbleChart" => group = None,
                _ if name.ends_with("Chart") => group = None,
                _ => {}
            },
        }
    }
    if groups.is_empty() {
        return None;
    }
    let combo = groups.contains(&ChartGroup::Bar) && groups.contains(&ChartGroup::Line);
    let mut kind = if histogram {
        ChartKind::Histogram
    } else if groups.contains(&ChartGroup::Unknown) {
        ChartKind::Unsupported
    } else if combo {
        ChartKind::Combo
    } else {
        match groups[0] {
            ChartGroup::Line => ChartKind::Line,
            ChartGroup::Area => ChartKind::Area,
            ChartGroup::Pie => ChartKind::Pie,
            ChartGroup::Donut => ChartKind::Donut,
            ChartGroup::Scatter => ChartKind::Scatter,
            ChartGroup::Bubble => ChartKind::Bubble,
            ChartGroup::Bar => match (bar_dir.as_str(), grouping.as_str()) {
                ("bar", "stacked") => ChartKind::BarStacked,
                ("bar", "percentStacked") => ChartKind::BarPct,
                ("bar", _) => ChartKind::Bar,
                (_, "stacked") => ChartKind::ColumnStacked,
                (_, "percentStacked") => ChartKind::ColumnPct,
                _ => ChartKind::Column,
            },
            ChartGroup::Unknown => ChartKind::Unsupported,
        }
    };

    let (category_axis, value_axis, parsed_secondary_axis) = parse_axes(bytes);
    let mut categories = None;
    let mut series = Vec::new();
    for (index, item) in raw.into_iter().enumerate() {
        if categories.is_none() {
            categories = item
                .categories
                .as_deref()
                .and_then(|formula| parse_chart_range(wb, sheet, formula));
        }
        let values = item
            .y
            .as_deref()
            .or(item.x.as_deref())
            .or(item.categories.as_deref())
            .and_then(|formula| parse_chart_range(wb, sheet, formula));
        let Some(values) = values else {
            continue;
        };
        let name = if !item.name.is_empty() {
            item.name
        } else {
            chart_label_from_formula(wb, sheet, &item.name_formula)
                .unwrap_or_else(|| format!("S{}", index + 1))
        };
        series.push(Series {
            name,
            values,
            x: item
                .x
                .as_deref()
                .and_then(|formula| parse_chart_range(wb, sheet, formula)),
            size: item
                .size
                .as_deref()
                .and_then(|formula| parse_chart_range(wb, sheet, formula)),
            color: item.color,
            secondary_axis: combo && item.group == ChartGroup::Line,
            trendline: item.trendline,
        });
    }
    if series.is_empty() {
        kind = ChartKind::Unsupported;
    }
    Some(Chart {
        id: ChartId::new(0),
        kind,
        title: (!title.is_empty()).then_some(title),
        categories,
        series,
        category_axis,
        value_axis,
        secondary_axis: (kind == ChartKind::Combo)
            .then(|| parsed_secondary_axis.unwrap_or_default()),
        legend: if saw_legend { legend } else { LegendPos::None },
        data_labels,
        anchor,
        sheet,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AxisTag {
    Category,
    Value,
}

struct RawAxis {
    tag: AxisTag,
    position: String,
    title: String,
    gridlines: bool,
}

fn parse_axes(bytes: &[u8]) -> (Axis, Axis, Option<Axis>) {
    let mut reader = XmlReader::new(bytes);
    let mut current: Option<RawAxis> = None;
    let mut in_title = false;
    let mut in_title_text = false;
    let mut axes = Vec::new();
    while let Ok(Some(event)) = reader.next() {
        match event {
            XmlEvent::Start { name, attrs } | XmlEvent::Empty { name, attrs } => {
                match name.as_str() {
                    "catAx" if current.is_none() => {
                        current = Some(RawAxis {
                            tag: AxisTag::Category,
                            position: String::new(),
                            title: String::new(),
                            gridlines: false,
                        });
                    }
                    "valAx" if current.is_none() => {
                        current = Some(RawAxis {
                            tag: AxisTag::Value,
                            position: String::new(),
                            title: String::new(),
                            gridlines: false,
                        });
                    }
                    "axPos" => {
                        if let Some(axis) = current.as_mut()
                            && let Some(position) = attr(&attrs, "val")
                        {
                            axis.position = position.to_string();
                        }
                    }
                    "majorGridlines" => {
                        if let Some(axis) = current.as_mut() {
                            axis.gridlines = true;
                        }
                    }
                    "title" if current.is_some() => in_title = true,
                    "t" | "v" if in_title => in_title_text = true,
                    _ => {}
                }
            }
            XmlEvent::Text(text) if in_title && in_title_text => {
                if let Some(axis) = current.as_mut() {
                    axis.title.push_str(&text);
                }
            }
            XmlEvent::Text(_) => {}
            XmlEvent::End { name } => match name.as_str() {
                "t" | "v" => in_title_text = false,
                "title" => in_title = false,
                "catAx" | "valAx" => {
                    if let Some(axis) = current.take() {
                        axes.push(axis);
                    }
                }
                _ => {}
            },
        }
    }

    let mut category = None;
    let mut primary = None;
    let mut secondary = None;
    for axis in axes {
        let modeled = Axis {
            title: (!axis.title.is_empty()).then_some(axis.title),
            gridlines: axis.gridlines,
        };
        match (axis.tag, axis.position.as_str()) {
            (AxisTag::Category, _) | (AxisTag::Value, "b" | "t") if category.is_none() => {
                category = Some(modeled);
            }
            (AxisTag::Value, "r") if secondary.is_none() => secondary = Some(modeled),
            (AxisTag::Value, _) if primary.is_none() => primary = Some(modeled),
            _ => {}
        }
    }
    (
        category.unwrap_or_default(),
        primary.unwrap_or_default(),
        secondary,
    )
}

fn chart_label_from_formula(
    wb: &Workbook,
    sheet: omacell_core::addr::SheetId,
    formula: &str,
) -> Option<String> {
    if formula.is_empty() {
        return None;
    }
    let range = parse_chart_range(wb, sheet, formula)?;
    let source_sheet = range.start.sheet.unwrap_or(sheet);
    let slot = wb
        .get(source_sheet, range.start.row, range.start.col)
        .ok()??;
    Some(match slot.value {
        Value::Text(id) => wb.intern().strings.get(id)?.to_string(),
        Value::Number(number) => number.to_string(),
        Value::Bool(value) => value.to_string(),
        _ => return None,
    })
}

fn parse_chart_range(
    wb: &Workbook,
    sheet: omacell_core::addr::SheetId,
    formula: &str,
) -> Option<omacell_core::addr::RangeRef> {
    let parsed = omacell_core::addr::parse_a1(formula.trim().trim_start_matches('=')).ok()?;
    let mut range = match wb.resolve_parsed(parsed).ok()? {
        omacell_core::addr::RefKind::Range(range) => range,
        omacell_core::addr::RefKind::Cell(cell) => {
            omacell_core::addr::RangeRef::from_corners(cell, cell)
        }
    };
    if range.start.sheet.is_none() {
        range.start.sheet = Some(sheet);
        range.end.sheet = Some(sheet);
    }
    Some(range)
}

/// Parse imported x14 sparkline groups into the core model.
pub(crate) fn parse_sparklines(
    blobs: &[Vec<u8>],
    wb: &Workbook,
    sheet: omacell_core::addr::SheetId,
) -> Vec<Sparkline> {
    let mut out = Vec::new();
    for blob in blobs {
        let mut reader = XmlReader::new(blob);
        let mut kind = SparklineKind::Line;
        let mut in_sparkline = false;
        let mut in_formula = false;
        let mut in_cell = false;
        let mut formula = String::new();
        let mut cell = String::new();
        while let Ok(Some(event)) = reader.next() {
            match event {
                XmlEvent::Start { name, attrs } | XmlEvent::Empty { name, attrs } => {
                    match name.as_str() {
                        "sparklineGroup" => {
                            kind = match attr(&attrs, "type") {
                                Some("column") => SparklineKind::Column,
                                Some("stacked") => SparklineKind::WinLoss,
                                _ => SparklineKind::Line,
                            };
                        }
                        "sparkline" => {
                            in_sparkline = true;
                            formula.clear();
                            cell.clear();
                        }
                        "f" if in_sparkline => in_formula = true,
                        "sqref" if in_sparkline => in_cell = true,
                        _ => {}
                    }
                }
                XmlEvent::Text(text) if in_formula => formula.push_str(&text),
                XmlEvent::Text(text) if in_cell => cell.push_str(&text),
                XmlEvent::Text(_) => {}
                XmlEvent::End { name } => match name.as_str() {
                    "f" => in_formula = false,
                    "sqref" => in_cell = false,
                    "sparkline" => {
                        in_sparkline = false;
                        if let (Some(data), Ok(target)) = (
                            parse_chart_range(wb, sheet, formula.trim()),
                            omacell_core::addr::parse_a1_cell(cell.trim()),
                        ) {
                            out.push(Sparkline {
                                kind,
                                data,
                                row: target.row,
                                col: target.col,
                                sheet,
                            });
                        }
                    }
                    _ => {}
                },
            }
        }
    }
    out
}

/// Whether raw x14 fragments describe exactly the current modeled sparklines.
pub(crate) fn sparkline_extras_match(blobs: &[Vec<u8>], wb: &Workbook, sheet: &Sheet) -> bool {
    parse_sparklines(blobs, wb, sheet.id) == sheet.sparklines
}

fn truthy(value: &str) -> bool {
    matches!(value, "1" | "true" | "True" | "TRUE")
}

#[cfg(test)]
mod tests {
    use super::*;
    use omacell_core::addr::{CellRef, RangeRef};
    use omacell_core::chart::{ChartKind, chart_from_range};

    #[test]
    fn generated_chart_parses_back_to_the_same_kind() {
        let mut wb = Workbook::new();
        let sheet = wb.active_sheet();
        wb.set_text(sheet, 0, 0, "Month").unwrap();
        wb.set_text(sheet, 0, 1, "East").unwrap();
        wb.set_number(sheet, 1, 0, 1.0).unwrap();
        wb.set_number(sheet, 1, 1, 42.0).unwrap();
        let chart = chart_from_range(
            &wb,
            sheet,
            RangeRef::from_corners(
                CellRef::new(0, 0).unwrap().on_sheet(sheet),
                CellRef::new(1, 1).unwrap().on_sheet(sheet),
            ),
            ChartKind::Column,
            Some("Sales".into()),
        )
        .unwrap();
        let bytes = chart_xml(&wb, wb.sheet(sheet).unwrap(), &chart).unwrap();
        let parsed = parse_chart_part(&bytes, &wb, sheet, chart.anchor).unwrap();
        assert_eq!(
            parsed.kind,
            ChartKind::Column,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        assert_eq!(parsed.series.len(), 1);
    }

    #[test]
    fn parser_keeps_formula_name_and_axis_properties() {
        let mut wb = Workbook::new();
        let sheet = wb.active_sheet();
        wb.set_text(sheet, 0, 0, "Month").unwrap();
        wb.set_text(sheet, 0, 1, "East").unwrap();
        wb.set_number(sheet, 1, 0, 1.0).unwrap();
        wb.set_number(sheet, 1, 1, 42.0).unwrap();
        let xml = br#"<c:chartSpace xmlns:c="chart" xmlns:a="drawing"><c:chart><c:plotArea><c:lineChart><c:ser><c:tx><c:strRef><c:f>'Sheet1'!$B$1</c:f></c:strRef></c:tx><c:cat><c:numRef><c:f>'Sheet1'!$A$2:$A$2</c:f></c:numRef></c:cat><c:val><c:numRef><c:f>'Sheet1'!$B$2:$B$2</c:f></c:numRef></c:val></c:ser></c:lineChart><c:catAx><c:axPos val="b"/><c:title><c:tx><c:rich><a:p><a:r><a:t>Month</a:t></a:r></a:p></c:rich></c:tx></c:title></c:catAx><c:valAx><c:axPos val="l"/><c:title><c:tx><c:rich><a:p><a:r><a:t>Sales</a:t></a:r></a:p></c:rich></c:tx></c:title><c:majorGridlines/></c:valAx></c:plotArea></c:chart></c:chartSpace>"#;

        let chart = parse_chart_part(xml, &wb, sheet, ChartAnchor::default()).unwrap();
        assert_eq!(chart.kind, ChartKind::Line);
        assert_eq!(chart.title, None, "axis title must not become chart title");
        assert_eq!(chart.series[0].name, "East");
        assert_eq!(chart.category_axis.title.as_deref(), Some("Month"));
        assert!(!chart.category_axis.gridlines);
        assert_eq!(chart.value_axis.title.as_deref(), Some("Sales"));
        assert!(chart.value_axis.gridlines);
    }
}
