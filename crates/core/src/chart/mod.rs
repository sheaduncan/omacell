//! Charts and in-cell sparklines (spec F-8).
//!
//! The model lives on [`crate::sheet::Sheet`]. Sampling reads the workbook;
//! [`scene::layout`] produces a toolkit-neutral scene that SVG, PNG, and the
//! GUI all consume.

mod model;
mod sample;
mod scene;
mod svg;

pub use model::{
    Axis, Chart, ChartAnchor, ChartId, ChartKind, ChartTheme, LegendPos, Series, Sparkline,
    SparklineKind, Trendline, TrendlineKind,
};
pub use sample::{SampledChart, SampledSeries, chart_from_range, parse_range, sample};
pub use scene::{Op, Scene, layout, layout_chart, layout_sparkline};
pub use svg::to_svg;
