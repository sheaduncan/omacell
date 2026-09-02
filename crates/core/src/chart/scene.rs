//! Toolkit-neutral 2-D scene. GUI, SVG, and PNG consume the same ops.

use super::model::{
    Chart, ChartKind, ChartTheme, LegendPos, Sparkline, SparklineKind, TrendlineKind,
};
use super::sample::{SampledChart, SampledSeries, sample};
use crate::error::CoreError;
use crate::workbook::Workbook;

/// One paint operation.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Filled rectangle.
    FillRect {
        /// Left.
        x: f32,
        /// Top.
        y: f32,
        /// Width.
        w: f32,
        /// Height.
        h: f32,
        /// `#rrggbb`.
        color: String,
    },
    /// Polyline.
    Polyline {
        /// Points.
        points: Vec<(f32, f32)>,
        /// Stroke.
        color: String,
        /// Width in scene units.
        width: f32,
    },
    /// Filled polygon.
    Polygon {
        /// Points.
        points: Vec<(f32, f32)>,
        /// Fill.
        color: String,
    },
    /// Circle.
    Circle {
        /// Center x.
        x: f32,
        /// Center y.
        y: f32,
        /// Radius.
        r: f32,
        /// Fill.
        color: String,
    },
    /// Text.
    Text {
        /// Anchor x.
        x: f32,
        /// Anchor y.
        y: f32,
        /// Content.
        text: String,
        /// Fill.
        color: String,
        /// Size in scene units.
        size: f32,
    },
}

/// Complete scene.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
    /// Paint list (back to front).
    pub ops: Vec<Op>,
}

impl Scene {
    fn new(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            ops: Vec::new(),
        }
    }
}

/// Layout a sampled chart.
#[must_use]
pub fn layout(
    sampled: &SampledChart,
    chart: &Chart,
    theme: &ChartTheme,
    width: f32,
    height: f32,
) -> Scene {
    let mut scene = Scene::new(width, height);
    scene.ops.push(Op::FillRect {
        x: 0.0,
        y: 0.0,
        w: width,
        h: height,
        color: theme.background.clone(),
    });
    let mut top = 16.0;
    if let Some(title) = sampled.title.as_deref().or(chart.title.as_deref()) {
        scene.ops.push(Op::Text {
            x: 16.0,
            y: 18.0,
            text: title.to_string(),
            color: theme.foreground.clone(),
            size: 14.0,
        });
        top = 32.0;
    }
    let legend_w = if chart.legend == LegendPos::Right {
        88.0
    } else {
        8.0
    };
    let plot = PlotRect {
        x: 48.0,
        y: top,
        w: (width - 48.0 - legend_w).max(40.0),
        h: (height - top - 36.0).max(40.0),
    };
    match sampled.kind {
        ChartKind::Pie | ChartKind::Donut => pie(
            &mut scene,
            sampled,
            theme,
            plot,
            sampled.kind == ChartKind::Donut,
        ),
        ChartKind::Scatter | ChartKind::Bubble => scatter(&mut scene, sampled, chart, theme, plot),
        ChartKind::Unsupported => unsupported(&mut scene, chart, theme, plot),
        _ => cartesian(&mut scene, sampled, chart, theme, plot),
    }
    if !matches!(
        sampled.kind,
        ChartKind::Pie | ChartKind::Donut | ChartKind::Unsupported
    ) {
        axis_titles(&mut scene, chart, theme, plot, height);
    }
    legend(&mut scene, sampled, chart, theme, plot);
    scene
}

fn axis_titles(scene: &mut Scene, chart: &Chart, theme: &ChartTheme, plot: PlotRect, height: f32) {
    if let Some(title) = chart.category_axis.title.as_deref() {
        scene.ops.push(Op::Text {
            x: plot.x + plot.w * 0.4,
            y: height - 6.0,
            text: title.to_string(),
            color: theme.foreground.clone(),
            size: 11.0,
        });
    }
    if let Some(title) = chart.value_axis.title.as_deref() {
        scene.ops.push(Op::Text {
            x: 4.0,
            y: plot.y + 14.0,
            text: title.to_string(),
            color: theme.foreground.clone(),
            size: 11.0,
        });
    }
    if let Some(title) = chart
        .secondary_axis
        .as_ref()
        .and_then(|axis| axis.title.as_deref())
    {
        scene.ops.push(Op::Text {
            x: plot.x + plot.w + 6.0,
            y: plot.y + 14.0,
            text: title.to_string(),
            color: theme.foreground.clone(),
            size: 11.0,
        });
    }
}

/// Sample and layout.
pub fn layout_chart(
    wb: &Workbook,
    chart: &Chart,
    theme: &ChartTheme,
    width: f32,
    height: f32,
) -> Result<Scene, CoreError> {
    Ok(layout(&sample(wb, chart)?, chart, theme, width, height))
}

/// Layout a sparkline into a cell-sized scene.
#[must_use]
pub fn layout_sparkline(
    values: &[f64],
    spark: &Sparkline,
    theme: &ChartTheme,
    w: f32,
    h: f32,
) -> Scene {
    let mut scene = Scene::new(w, h);
    let color = theme.series_color(0, None);
    let finite: Vec<(usize, f64)> = values
        .iter()
        .enumerate()
        .filter(|(_, v)| v.is_finite())
        .map(|(i, v)| (i, *v))
        .collect();
    if finite.is_empty() {
        return scene;
    }
    match spark.kind {
        SparklineKind::WinLoss => {
            let bar_w = (w / values.len().max(1) as f32).max(1.0);
            let mid = h * 0.5;
            for (i, v) in finite {
                let bh = h * 0.4;
                let y = if v >= 0.0 { mid - bh } else { mid };
                scene.ops.push(Op::FillRect {
                    x: i as f32 * bar_w,
                    y,
                    w: bar_w * 0.8,
                    h: bh,
                    color: color.to_string(),
                });
            }
        }
        SparklineKind::Column => {
            let min = finite.iter().map(|(_, v)| *v).fold(f64::INFINITY, f64::min);
            let max = finite
                .iter()
                .map(|(_, v)| *v)
                .fold(f64::NEG_INFINITY, f64::max);
            let span = (max - min).max(1e-9);
            let bar_w = (w / values.len().max(1) as f32).max(1.0);
            for (i, v) in finite {
                let t = ((v - min) / span) as f32;
                let bh = (t * h).max(1.0);
                scene.ops.push(Op::FillRect {
                    x: i as f32 * bar_w,
                    y: h - bh,
                    w: bar_w * 0.8,
                    h: bh,
                    color: color.to_string(),
                });
            }
        }
        SparklineKind::Line => {
            let min = finite.iter().map(|(_, v)| *v).fold(f64::INFINITY, f64::min);
            let max = finite
                .iter()
                .map(|(_, v)| *v)
                .fold(f64::NEG_INFINITY, f64::max);
            let span = (max - min).max(1e-9);
            let n = values.len().max(1) as f32;
            let points: Vec<(f32, f32)> = finite
                .iter()
                .map(|(i, v)| {
                    let x = (*i as f32 + 0.5) * w / n;
                    let y = h - ((*v - min) / span) as f32 * h;
                    (x, y)
                })
                .collect();
            scene.ops.push(Op::Polyline {
                points,
                color: color.to_string(),
                width: 1.2,
            });
        }
    }
    scene
}

#[derive(Clone, Copy)]
struct PlotRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

fn cartesian(
    scene: &mut Scene,
    sampled: &SampledChart,
    chart: &Chart,
    theme: &ChartTheme,
    plot: PlotRect,
) {
    let cats = sampled
        .series
        .first()
        .map(|s| s.categories.clone())
        .unwrap_or_default();
    let n = cats.len().max(1);
    let (ymin, ymax) = y_bounds_for(sampled, chart.kind, None);
    grid(scene, theme, &plot, ymin, ymax, chart.value_axis.gridlines);
    scene.ops.push(Op::Polyline {
        points: vec![
            (plot.x, plot.y),
            (plot.x, plot.y + plot.h),
            (plot.x + plot.w, plot.y + plot.h),
        ],
        color: theme.axis.clone(),
        width: 1.0,
    });
    for (i, label) in cats.iter().enumerate() {
        let x = plot.x + (i as f32 + 0.5) * plot.w / n as f32;
        scene.ops.push(Op::Text {
            x,
            y: plot.y + plot.h + 12.0,
            text: label.clone(),
            color: theme.axis.clone(),
            size: 9.0,
        });
    }
    match chart.kind {
        ChartKind::Line | ChartKind::Combo => {
            if chart.kind == ChartKind::Combo {
                bars(scene, sampled, theme, &plot, ymin, ymax, false);
                let (secondary_min, secondary_max) = y_bounds_for(sampled, chart.kind, Some(true));
                secondary_axis(scene, theme, &plot, secondary_min, secondary_max);
                lines(
                    scene,
                    sampled,
                    chart,
                    theme,
                    &plot,
                    secondary_min,
                    secondary_max,
                );
            } else {
                lines(scene, sampled, chart, theme, &plot, ymin, ymax);
            }
        }
        ChartKind::Area => area(scene, sampled, theme, &plot, ymin, ymax),
        _ => bars(
            scene,
            sampled,
            theme,
            &plot,
            ymin,
            ymax,
            chart.kind.horizontal(),
        ),
    }
}

fn unsupported(scene: &mut Scene, chart: &Chart, theme: &ChartTheme, plot: PlotRect) {
    scene.ops.push(Op::Polyline {
        points: vec![
            (plot.x, plot.y),
            (plot.x + plot.w, plot.y),
            (plot.x + plot.w, plot.y + plot.h),
            (plot.x, plot.y + plot.h),
            (plot.x, plot.y),
        ],
        color: theme.axis.clone(),
        width: 1.0,
    });
    scene.ops.push(Op::Text {
        x: plot.x + 12.0,
        y: plot.y + 28.0,
        text: "Unsupported chart (preserved in .xlsx)".into(),
        color: theme.foreground.clone(),
        size: 12.0,
    });
    let mut ranges = chart
        .series
        .iter()
        .map(|series| series.values.to_a1())
        .collect::<Vec<_>>();
    ranges.sort();
    ranges.dedup();
    if !ranges.is_empty() {
        scene.ops.push(Op::Text {
            x: plot.x + 12.0,
            y: plot.y + 48.0,
            text: ranges.join(", "),
            color: theme.axis.clone(),
            size: 9.0,
        });
    }
}

fn scatter(
    scene: &mut Scene,
    sampled: &SampledChart,
    chart: &Chart,
    theme: &ChartTheme,
    plot: PlotRect,
) {
    let (xmin, xmax, ymin, ymax) = xy_bounds(sampled);
    grid(scene, theme, &plot, ymin, ymax, chart.value_axis.gridlines);
    scene.ops.push(Op::Polyline {
        points: vec![
            (plot.x, plot.y),
            (plot.x, plot.y + plot.h),
            (plot.x + plot.w, plot.y + plot.h),
        ],
        color: theme.axis.clone(),
        width: 1.0,
    });
    let bubble = sampled.kind == ChartKind::Bubble;
    for (si, series) in sampled.series.iter().enumerate() {
        let color = theme.series_color(si, series.color.as_deref());
        for i in 0..series.y.len() {
            if !series.x[i].is_finite() || !series.y[i].is_finite() {
                continue;
            }
            let x = map(series.x[i], xmin, xmax, plot.x, plot.x + plot.w);
            let y = map(series.y[i], ymin, ymax, plot.y + plot.h, plot.y);
            let r = if bubble {
                (series.size[i].abs() as f32).clamp(2.0, 18.0)
            } else {
                3.0
            };
            scene.ops.push(Op::Circle {
                x,
                y,
                r,
                color: color.to_string(),
            });
        }
        trendline(
            scene, series, chart, theme, si, &plot, xmin, xmax, ymin, ymax,
        );
    }
}

fn pie(scene: &mut Scene, sampled: &SampledChart, theme: &ChartTheme, plot: PlotRect, donut: bool) {
    let series = match sampled.series.first() {
        Some(s) => s,
        None => return,
    };
    let total: f64 = series
        .y
        .iter()
        .copied()
        .filter(|v| v.is_finite() && *v > 0.0)
        .sum();
    if total <= 0.0 {
        return;
    }
    let cx = plot.x + plot.w * 0.5;
    let cy = plot.y + plot.h * 0.5;
    let r = plot.w.min(plot.h) * 0.42;
    let mut a0 = -std::f32::consts::FRAC_PI_2;
    for (i, v) in series.y.iter().enumerate() {
        if !v.is_finite() || *v <= 0.0 {
            continue;
        }
        let sweep = (*v / total) as f32 * std::f32::consts::TAU;
        let color = theme.series_color(i, series.color.as_deref());
        let mut pts = vec![(cx, cy)];
        let steps = ((sweep.abs() * 24.0).ceil() as usize).max(2);
        for s in 0..=steps {
            let a = a0 + sweep * s as f32 / steps as f32;
            pts.push((cx + r * a.cos(), cy + r * a.sin()));
        }
        scene.ops.push(Op::Polygon {
            points: pts,
            color: color.to_string(),
        });
        a0 += sweep;
    }
    if donut {
        scene.ops.push(Op::Circle {
            x: cx,
            y: cy,
            r: r * 0.45,
            color: theme.background.clone(),
        });
    }
}

fn bars(
    scene: &mut Scene,
    sampled: &SampledChart,
    theme: &ChartTheme,
    plot: &PlotRect,
    ymin: f64,
    ymax: f64,
    horizontal: bool,
) {
    let n = sampled
        .series
        .first()
        .map(|s| s.y.len())
        .unwrap_or(0)
        .max(1);
    let k = sampled.series.len().max(1);
    let stacked = sampled.kind.stacked() || sampled.kind.percent();
    let mut acc_pos = vec![0.0; n];
    let mut acc_neg = vec![0.0; n];
    for (si, series) in sampled.series.iter().enumerate() {
        let color = theme.series_color(si, series.color.as_deref());
        if sampled.kind == ChartKind::Combo && series.secondary_axis {
            continue;
        }
        for i in 0..n {
            let mut v = series.y.get(i).copied().unwrap_or(f64::NAN);
            if !v.is_finite() {
                continue;
            }
            if sampled.kind.percent() {
                let tot: f64 = sampled
                    .series
                    .iter()
                    .map(|s| s.y.get(i).copied().unwrap_or(0.0).abs())
                    .sum();
                if tot > 0.0 {
                    v = v / tot * 100.0;
                }
            }
            let (base, span) = if stacked {
                if v >= 0.0 {
                    let b = acc_pos[i];
                    acc_pos[i] += v;
                    (b, v)
                } else {
                    let b = acc_neg[i];
                    acc_neg[i] += v;
                    (b, v)
                }
            } else {
                (0.0, v)
            };
            let group = i as f32;
            if horizontal {
                let y = plot.y + (group + 0.1) * plot.h / n as f32;
                let h = plot.h / n as f32 * if stacked { 0.8 } else { 0.8 / k as f32 };
                let yy = if stacked { y } else { y + si as f32 * h };
                let x0 = map(base, ymin, ymax, plot.x, plot.x + plot.w);
                let x1 = map(base + span, ymin, ymax, plot.x, plot.x + plot.w);
                scene.ops.push(Op::FillRect {
                    x: x0.min(x1),
                    y: yy,
                    w: (x1 - x0).abs().max(1.0),
                    h,
                    color: color.to_string(),
                });
            } else {
                let x = plot.x + (group + 0.1) * plot.w / n as f32;
                let w = plot.w / n as f32 * if stacked { 0.8 } else { 0.8 / k as f32 };
                let xx = if stacked { x } else { x + si as f32 * w };
                let y0 = map(base, ymin, ymax, plot.y + plot.h, plot.y);
                let y1 = map(base + span, ymin, ymax, plot.y + plot.h, plot.y);
                scene.ops.push(Op::FillRect {
                    x: xx,
                    y: y0.min(y1),
                    w,
                    h: (y1 - y0).abs().max(1.0),
                    color: color.to_string(),
                });
            }
        }
    }
}

fn lines(
    scene: &mut Scene,
    sampled: &SampledChart,
    chart: &Chart,
    theme: &ChartTheme,
    plot: &PlotRect,
    ymin: f64,
    ymax: f64,
) {
    let n = sampled
        .series
        .first()
        .map(|s| s.y.len())
        .unwrap_or(0)
        .max(1);
    for (si, series) in sampled.series.iter().enumerate() {
        if sampled.kind == ChartKind::Combo
            && !series.secondary_axis
            && sampled.kind != ChartKind::Line
        {
            // combo: line series is the secondary one; also plot remaining as lines if Line kind
        }
        if sampled.kind == ChartKind::Combo && !series.secondary_axis {
            continue;
        }
        let color = theme.series_color(si, series.color.as_deref());
        let mut points = Vec::new();
        for i in 0..n {
            let v = series.y.get(i).copied().unwrap_or(f64::NAN);
            if !v.is_finite() {
                if points.len() > 1 {
                    scene.ops.push(Op::Polyline {
                        points: std::mem::take(&mut points),
                        color: color.to_string(),
                        width: 2.0,
                    });
                } else {
                    points.clear();
                }
                continue;
            }
            let x = plot.x + (i as f32 + 0.5) * plot.w / n as f32;
            let y = map(v, ymin, ymax, plot.y + plot.h, plot.y);
            points.push((x, y));
        }
        if points.len() > 1 {
            scene.ops.push(Op::Polyline {
                points,
                color: color.to_string(),
                width: 2.0,
            });
        }
        let xmin = 0.0;
        let xmax = (n.saturating_sub(1)) as f64;
        trendline(
            scene, series, chart, theme, si, plot, xmin, xmax, ymin, ymax,
        );
    }
}

fn area(
    scene: &mut Scene,
    sampled: &SampledChart,
    theme: &ChartTheme,
    plot: &PlotRect,
    ymin: f64,
    ymax: f64,
) {
    let n = sampled
        .series
        .first()
        .map(|s| s.y.len())
        .unwrap_or(0)
        .max(1);
    let mut acc = vec![0.0; n];
    for (si, series) in sampled.series.iter().enumerate() {
        let color = theme.series_color(si, series.color.as_deref());
        let mut top = Vec::new();
        for (i, slot) in acc.iter_mut().enumerate() {
            let v = series
                .y
                .get(i)
                .copied()
                .filter(|v| v.is_finite())
                .unwrap_or(0.0);
            *slot += v;
            let x = plot.x + (i as f32 + 0.5) * plot.w / n as f32;
            let y = map(*slot, ymin, ymax, plot.y + plot.h, plot.y);
            top.push((x, y));
        }
        let mut pts = top.clone();
        for i in (0..n).rev() {
            let base = acc[i]
                - series
                    .y
                    .get(i)
                    .copied()
                    .filter(|v| v.is_finite())
                    .unwrap_or(0.0);
            let x = plot.x + (i as f32 + 0.5) * plot.w / n as f32;
            let y = map(base, ymin, ymax, plot.y + plot.h, plot.y);
            pts.push((x, y));
        }
        scene.ops.push(Op::Polygon {
            points: pts,
            color: color.to_string(),
        });
        scene.ops.push(Op::Polyline {
            points: top,
            color: color.to_string(),
            width: 1.5,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn trendline(
    scene: &mut Scene,
    series: &SampledSeries,
    chart: &Chart,
    theme: &ChartTheme,
    si: usize,
    plot: &PlotRect,
    xmin: f64,
    xmax: f64,
    ymin: f64,
    ymax: f64,
) {
    let Some(tr) = chart.series.get(si).and_then(|s| s.trendline.as_ref()) else {
        return;
    };
    let pts: Vec<(f64, f64)> = series
        .x
        .iter()
        .zip(&series.y)
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .map(|(x, y)| (*x, *y))
        .collect();
    if pts.len() < 2 {
        return;
    }
    let color = theme.series_color(si, series.color.as_deref()).to_string();
    let mut line = Vec::new();
    match tr.kind {
        TrendlineKind::Linear => {
            let (a, b) = linear_fit(&pts);
            line.push((
                map(xmin, xmin, xmax, plot.x, plot.x + plot.w),
                map(a + b * xmin, ymin, ymax, plot.y + plot.h, plot.y),
            ));
            line.push((
                map(xmax, xmin, xmax, plot.x, plot.x + plot.w),
                map(a + b * xmax, ymin, ymax, plot.y + plot.h, plot.y),
            ));
        }
        TrendlineKind::Exponential => {
            let logged: Vec<(f64, f64)> = pts
                .iter()
                .filter(|(_, y)| *y > 0.0)
                .map(|(x, y)| (*x, y.ln()))
                .collect();
            if logged.len() < 2 {
                return;
            }
            let (a, b) = linear_fit(&logged);
            for i in 0..24 {
                let x = xmin + (xmax - xmin) * i as f64 / 23.0;
                let y = (a + b * x).exp();
                line.push((
                    map(x, xmin, xmax, plot.x, plot.x + plot.w),
                    map(y, ymin, ymax, plot.y + plot.h, plot.y),
                ));
            }
        }
        TrendlineKind::MovingAverage => {
            let period = tr.period.max(2) as usize;
            if pts.len() < period {
                return;
            }
            for i in period - 1..pts.len() {
                let avg =
                    pts[i + 1 - period..=i].iter().map(|(_, y)| *y).sum::<f64>() / period as f64;
                let x = pts[i].0;
                line.push((
                    map(x, xmin, xmax, plot.x, plot.x + plot.w),
                    map(avg, ymin, ymax, plot.y + plot.h, plot.y),
                ));
            }
        }
    }
    if line.len() > 1 {
        scene.ops.push(Op::Polyline {
            points: line,
            color,
            width: 1.5,
        });
    }
}

fn linear_fit(pts: &[(f64, f64)]) -> (f64, f64) {
    let n = pts.len() as f64;
    let sx: f64 = pts.iter().map(|(x, _)| *x).sum();
    let sy: f64 = pts.iter().map(|(_, y)| *y).sum();
    let sxx: f64 = pts.iter().map(|(x, _)| x * x).sum();
    let sxy: f64 = pts.iter().map(|(x, y)| x * y).sum();
    let den = n * sxx - sx * sx;
    if den.abs() < 1e-12 {
        return (sy / n, 0.0);
    }
    let b = (n * sxy - sx * sy) / den;
    let a = (sy - b * sx) / n;
    (a, b)
}

fn legend(
    scene: &mut Scene,
    sampled: &SampledChart,
    chart: &Chart,
    theme: &ChartTheme,
    plot: PlotRect,
) {
    if chart.legend == LegendPos::None {
        return;
    }
    let names: Vec<(usize, String)> = if matches!(sampled.kind, ChartKind::Pie | ChartKind::Donut) {
        sampled
            .series
            .first()
            .map(|s| {
                s.categories
                    .iter()
                    .enumerate()
                    .map(|(i, c)| (i, c.clone()))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        sampled
            .series
            .iter()
            .enumerate()
            .map(|(i, s)| (i, s.name.clone()))
            .collect()
    };
    for (i, (idx, name)) in names.iter().enumerate() {
        let color = theme.series_color(*idx, None);
        let (x, y) = if chart.legend == LegendPos::Bottom {
            (plot.x + i as f32 * 72.0, plot.y + plot.h + 22.0)
        } else {
            (plot.x + plot.w + 8.0, plot.y + 8.0 + i as f32 * 16.0)
        };
        scene.ops.push(Op::FillRect {
            x,
            y: y - 8.0,
            w: 10.0,
            h: 10.0,
            color: color.to_string(),
        });
        scene.ops.push(Op::Text {
            x: x + 14.0,
            y,
            text: name.clone(),
            color: theme.foreground.clone(),
            size: 10.0,
        });
    }
}

fn grid(scene: &mut Scene, theme: &ChartTheme, plot: &PlotRect, ymin: f64, ymax: f64, on: bool) {
    if !on {
        return;
    }
    for i in 0..=4 {
        let v = ymin + (ymax - ymin) * i as f64 / 4.0;
        let y = map(v, ymin, ymax, plot.y + plot.h, plot.y);
        scene.ops.push(Op::Polyline {
            points: vec![(plot.x, y), (plot.x + plot.w, y)],
            color: theme.gridline.clone(),
            width: 1.0,
        });
        scene.ops.push(Op::Text {
            x: 8.0,
            y: y + 4.0,
            text: format!("{v:.1}"),
            color: theme.axis.clone(),
            size: 9.0,
        });
    }
}

fn secondary_axis(scene: &mut Scene, theme: &ChartTheme, plot: &PlotRect, ymin: f64, ymax: f64) {
    scene.ops.push(Op::Polyline {
        points: vec![
            (plot.x + plot.w, plot.y),
            (plot.x + plot.w, plot.y + plot.h),
        ],
        color: theme.axis.clone(),
        width: 1.0,
    });
    for i in 0..=4 {
        let value = ymin + (ymax - ymin) * i as f64 / 4.0;
        let y = map(value, ymin, ymax, plot.y + plot.h, plot.y);
        scene.ops.push(Op::Text {
            x: plot.x + plot.w + 4.0,
            y: y + 4.0,
            text: format!("{value:.1}"),
            color: theme.axis.clone(),
            size: 9.0,
        });
    }
}

fn y_bounds_for(sampled: &SampledChart, kind: ChartKind, secondary: Option<bool>) -> (f64, f64) {
    if kind.percent() {
        return (0.0, 100.0);
    }
    let mut min: f64 = 0.0;
    let mut max: f64 = 1.0;
    if kind.stacked() {
        let n = sampled.series.first().map(|s| s.y.len()).unwrap_or(0);
        for i in 0..n {
            let mut pos = 0.0;
            let mut neg = 0.0;
            for s in &sampled.series {
                if secondary.is_some_and(|wanted| s.secondary_axis != wanted) {
                    continue;
                }
                let v = s.y.get(i).copied().unwrap_or(0.0);
                if v.is_finite() {
                    if v >= 0.0 {
                        pos += v;
                    } else {
                        neg += v;
                    }
                }
            }
            min = min.min(neg);
            max = max.max(pos);
        }
    } else {
        for s in &sampled.series {
            if secondary.is_some_and(|wanted| s.secondary_axis != wanted) {
                continue;
            }
            for v in &s.y {
                if v.is_finite() {
                    min = min.min(*v);
                    max = max.max(*v);
                }
            }
        }
    }
    if (max - min).abs() < 1e-9 {
        max = min + 1.0;
    }
    (min, max)
}

fn xy_bounds(sampled: &SampledChart) -> (f64, f64, f64, f64) {
    let mut xmin: f64 = 0.0;
    let mut xmax: f64 = 1.0;
    let mut ymin: f64 = 0.0;
    let mut ymax: f64 = 1.0;
    for s in &sampled.series {
        for v in &s.x {
            if v.is_finite() {
                xmin = xmin.min(*v);
                xmax = xmax.max(*v);
            }
        }
        for v in &s.y {
            if v.is_finite() {
                ymin = ymin.min(*v);
                ymax = ymax.max(*v);
            }
        }
    }
    if (xmax - xmin).abs() < 1e-9 {
        xmax = xmin + 1.0;
    }
    if (ymax - ymin).abs() < 1e-9 {
        ymax = ymin + 1.0;
    }
    (xmin, xmax, ymin, ymax)
}

fn map(v: f64, vmin: f64, vmax: f64, a: f32, b: f32) -> f32 {
    let t = ((v - vmin) / (vmax - vmin).max(1e-12)) as f32;
    a + t * (b - a)
}
