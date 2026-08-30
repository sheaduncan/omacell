//! Sample chart series from a workbook.

use crate::addr::{RangeRef, SheetId, parse_a1};
use crate::error::CoreError;
use crate::value::Value;
use crate::workbook::Workbook;

use super::model::{Chart, ChartKind, Series};

/// Maximum cells accepted by one sampled chart range.
pub const MAX_CHART_POINTS: u64 = 1_000_000;

/// One sampled series ready to plot.
#[derive(Clone, Debug, PartialEq)]
pub struct SampledSeries {
    /// Legend name.
    pub name: String,
    /// Category labels (same length as `y`, padded).
    pub categories: Vec<String>,
    /// Y values (`NaN` = gap).
    pub y: Vec<f64>,
    /// Scatter/bubble X (`NaN` = gap).
    pub x: Vec<f64>,
    /// Bubble sizes.
    pub size: Vec<f64>,
    /// Palette override.
    pub color: Option<String>,
    /// Secondary axis.
    pub secondary_axis: bool,
}

/// Sampled chart payload (recomputed after recalc).
#[derive(Clone, Debug, PartialEq)]
pub struct SampledChart {
    /// Kind.
    pub kind: ChartKind,
    /// Title.
    pub title: Option<String>,
    /// Series.
    pub series: Vec<SampledSeries>,
}

/// Sample `chart` against `wb`.
pub fn sample(wb: &Workbook, chart: &Chart) -> Result<SampledChart, CoreError> {
    let sheet = chart.sheet;
    let categories = match chart.categories {
        Some(range) => read_labels(wb, sheet, range)?,
        None => Vec::new(),
    };
    let mut series = Vec::with_capacity(chart.series.len());
    for spec in &chart.series {
        series.push(sample_series(wb, sheet, spec, &categories)?);
    }
    if chart.kind == ChartKind::Histogram && series.len() == 1 {
        series[0] = histogram_bins(&series[0]);
    }
    Ok(SampledChart {
        kind: chart.kind,
        title: chart.title.clone(),
        series,
    })
}

fn sample_series(
    wb: &Workbook,
    sheet: SheetId,
    spec: &Series,
    categories: &[String],
) -> Result<SampledSeries, CoreError> {
    let y = read_numbers(wb, sheet, spec.values)?;
    let x = match spec.x {
        Some(range) => read_numbers(wb, sheet, range)?,
        None => (0..y.len()).map(|i| i as f64).collect(),
    };
    let size = match spec.size {
        Some(range) => read_numbers(wb, sheet, range)?,
        None => vec![1.0; y.len()],
    };
    let n = y.len().max(x.len()).max(size.len()).max(categories.len());
    let mut cats = categories.to_vec();
    cats.resize(n, String::new());
    for (i, slot) in cats.iter_mut().enumerate() {
        if slot.is_empty() {
            *slot = (i + 1).to_string();
        }
    }
    Ok(SampledSeries {
        name: spec.name.clone(),
        categories: cats,
        y: pad_nans(y, n),
        x: pad_nans(x, n),
        size: pad_nans(size, n),
        color: spec.color.clone(),
        secondary_axis: spec.secondary_axis,
    })
}

fn histogram_bins(src: &SampledSeries) -> SampledSeries {
    let values: Vec<f64> = src.y.iter().copied().filter(|v| v.is_finite()).collect();
    if values.is_empty() {
        return src.clone();
    }
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let bins = 8usize;
    let span = (max - min).max(1e-9);
    let mut counts = vec![0.0; bins];
    let mut labels = Vec::with_capacity(bins);
    for i in 0..bins {
        let lo = min + span * i as f64 / bins as f64;
        labels.push(format!("{lo:.3}"));
    }
    for v in values {
        let mut idx = ((v - min) / span * bins as f64).floor() as usize;
        if idx >= bins {
            idx = bins - 1;
        }
        counts[idx] += 1.0;
    }
    SampledSeries {
        name: src.name.clone(),
        categories: labels,
        y: counts,
        x: (0..bins).map(|i| i as f64).collect(),
        size: vec![1.0; bins],
        color: src.color.clone(),
        secondary_axis: false,
    }
}

fn pad_nans(mut v: Vec<f64>, n: usize) -> Vec<f64> {
    v.resize(n, f64::NAN);
    v
}

fn read_numbers(wb: &Workbook, sheet: SheetId, range: RangeRef) -> Result<Vec<f64>, CoreError> {
    let (r0, c0, r1, c1) = corners(range);
    enforce_point_limit(r0, c0, r1, c1)?;
    let sheet = range.start.sheet.unwrap_or(sheet);
    let mut out = Vec::new();
    for row in r0..=r1 {
        for col in c0..=c1 {
            out.push(match wb.get(sheet, row, col)? {
                Some(slot) => match slot.value {
                    Value::Number(n) => n,
                    Value::Bool(true) => 1.0,
                    Value::Bool(false) => 0.0,
                    _ => f64::NAN,
                },
                None => f64::NAN,
            });
        }
    }
    Ok(out)
}

fn read_labels(wb: &Workbook, sheet: SheetId, range: RangeRef) -> Result<Vec<String>, CoreError> {
    let (r0, c0, r1, c1) = corners(range);
    enforce_point_limit(r0, c0, r1, c1)?;
    let sheet = range.start.sheet.unwrap_or(sheet);
    let mut out = Vec::new();
    for row in r0..=r1 {
        for col in c0..=c1 {
            let label = match wb.get(sheet, row, col)? {
                Some(slot) => match slot.value {
                    Value::Text(id) => wb.intern().strings.get(id).unwrap_or("").to_string(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(true) => "TRUE".into(),
                    Value::Bool(false) => "FALSE".into(),
                    Value::Empty => String::new(),
                    Value::Error(kind) => kind.as_str().to_string(),
                    Value::Array(_) => String::new(),
                },
                None => String::new(),
            };
            out.push(label);
        }
    }
    Ok(out)
}

fn corners(range: RangeRef) -> (u32, u16, u32, u16) {
    (
        range.start.row.min(range.end.row),
        range.start.col.min(range.end.col),
        range.start.row.max(range.end.row),
        range.start.col.max(range.end.col),
    )
}

fn enforce_point_limit(r0: u32, c0: u16, r1: u32, c1: u16) -> Result<(), CoreError> {
    let rows = u64::from(r1 - r0) + 1;
    let cols = u64::from(c1 - c0) + 1;
    let points = rows.saturating_mul(cols);
    if points > MAX_CHART_POINTS {
        return Err(CoreError::new(
            "chart.limit",
            format!("chart range has {points} cells; maximum is {MAX_CHART_POINTS}"),
        )
        .with_hint("select a smaller range or aggregate the data before charting"));
    }
    Ok(())
}

/// Build a chart covering `range` using Excel-ish header detection.
pub fn chart_from_range(
    wb: &Workbook,
    sheet: SheetId,
    range: RangeRef,
    kind: ChartKind,
    title: Option<String>,
) -> Result<Chart, CoreError> {
    let (r0, c0, r1, c1) = corners(range);
    enforce_point_limit(r0, c0, r1, c1)?;
    if r0 == r1 && c0 == c1 {
        return Err(CoreError::new(
            crate::error::codes::ADDR_REF,
            "chart.fromselection needs more than one cell",
        ));
    }
    let header_row = if r0 < r1 && c0 < c1 {
        let mut has_series_header = false;
        for col in c0.saturating_add(1)..=c1 {
            if wb
                .get(sheet, r0, col)?
                .is_some_and(|slot| matches!(slot.value, Value::Text(_)))
            {
                has_series_header = true;
                break;
            }
        }
        has_series_header
    } else {
        false
    };
    let data_row0 = if header_row { r0 + 1 } else { r0 };
    if data_row0 > r1 {
        return Err(CoreError::new(
            "chart.range",
            "chart selection contains headers but no data rows",
        ));
    }
    let cat_col = c0;
    let mut series = Vec::new();
    if matches!(kind, ChartKind::Scatter | ChartKind::Bubble) {
        let first_numeric_col = if header_row { c0.saturating_add(1) } else { c0 };
        if first_numeric_col < c1 {
            let x = column_range(data_row0, r1, first_numeric_col, sheet)?;
            if kind == ChartKind::Bubble {
                let y_col = first_numeric_col.saturating_add(1);
                series.push(Series {
                    name: series_name(wb, sheet, r0, y_col, header_row, 1)?,
                    values: column_range(data_row0, r1, y_col, sheet)?,
                    x: Some(x),
                    size: (y_col < c1)
                        .then(|| column_range(data_row0, r1, y_col + 1, sheet))
                        .transpose()?,
                    color: None,
                    secondary_axis: false,
                    trendline: None,
                });
            } else {
                for col in first_numeric_col.saturating_add(1)..=c1 {
                    series.push(Series {
                        name: series_name(wb, sheet, r0, col, header_row, series.len() + 1)?,
                        values: column_range(data_row0, r1, col, sheet)?,
                        x: Some(x),
                        size: None,
                        color: None,
                        secondary_axis: false,
                        trendline: None,
                    });
                }
            }
        } else {
            series.push(Series {
                name: series_name(wb, sheet, r0, first_numeric_col, header_row, 1)?,
                values: column_range(data_row0, r1, first_numeric_col, sheet)?,
                x: None,
                size: None,
                color: None,
                secondary_axis: false,
                trendline: None,
            });
        }
    } else {
        for col in c0.saturating_add(1)..=c1 {
            series.push(Series {
                name: series_name(wb, sheet, r0, col, header_row, series.len() + 1)?,
                values: column_range(data_row0, r1, col, sheet)?,
                x: None,
                size: None,
                color: None,
                secondary_axis: kind == ChartKind::Combo && series.len() == 1,
                trendline: None,
            });
        }
    }
    if series.is_empty() {
        series.push(Series {
            name: "S1".into(),
            values: range,
            x: None,
            size: None,
            color: None,
            secondary_axis: false,
            trendline: None,
        });
    }
    if kind == ChartKind::Histogram {
        series.truncate(1);
    }
    let categories = if matches!(kind, ChartKind::Scatter | ChartKind::Bubble) {
        None
    } else {
        Some(RangeRef::from_corners(
            crate::addr::CellRef::new(data_row0, cat_col)?.on_sheet(sheet),
            crate::addr::CellRef::new(r1, cat_col)?.on_sheet(sheet),
        ))
    };
    Ok(Chart {
        id: super::model::ChartId::new(0),
        kind,
        title,
        categories,
        series,
        category_axis: super::model::Axis::default(),
        value_axis: super::model::Axis::default(),
        secondary_axis: kind.eq(&ChartKind::Combo).then(super::model::Axis::default),
        legend: super::model::LegendPos::Right,
        data_labels: false,
        anchor: super::model::ChartAnchor::default(),
        sheet,
    })
}

fn column_range(row0: u32, row1: u32, col: u16, sheet: SheetId) -> Result<RangeRef, CoreError> {
    Ok(RangeRef::from_corners(
        crate::addr::CellRef::new(row0, col)?.on_sheet(sheet),
        crate::addr::CellRef::new(row1, col)?.on_sheet(sheet),
    ))
}

fn series_name(
    wb: &Workbook,
    sheet: SheetId,
    header_row: u32,
    col: u16,
    has_header: bool,
    fallback_index: usize,
) -> Result<String, CoreError> {
    if has_header
        && let Some(slot) = wb.get(sheet, header_row, col)?
        && let Value::Text(id) = slot.value
    {
        return Ok(wb.intern().strings.get(id).unwrap_or("").to_string());
    }
    Ok(format!("S{fallback_index}"))
}

/// Parse an A1 range in `sheet`.
pub fn parse_range(wb: &Workbook, text: &str) -> Result<(SheetId, RangeRef), CoreError> {
    let parsed = parse_a1(text)?;
    match wb.resolve_parsed(parsed)? {
        crate::addr::RefKind::Range(range) => {
            let sheet = range.start.sheet.unwrap_or(wb.active_sheet());
            Ok((sheet, range))
        }
        crate::addr::RefKind::Cell(cell) => {
            let sheet = cell.sheet.unwrap_or(wb.active_sheet());
            Ok((sheet, RangeRef::from_corners(cell, cell)))
        }
    }
}
