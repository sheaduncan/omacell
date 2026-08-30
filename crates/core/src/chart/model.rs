//! Chart and sparkline records stored on a sheet.

use serde::{Deserialize, Serialize};

use crate::addr::{RangeRef, SheetId};

/// Stable chart id within a workbook.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChartId(u32);

impl ChartId {
    /// Wrap an index.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Numeric id.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// Core chart kinds (spec F-8.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    /// Line.
    Line,
    /// Clustered column.
    Column,
    /// Clustered bar.
    Bar,
    /// Stacked column.
    ColumnStacked,
    /// Stacked bar.
    BarStacked,
    /// 100% stacked column.
    ColumnPct,
    /// 100% stacked bar.
    BarPct,
    /// Area.
    Area,
    /// Pie.
    Pie,
    /// Donut.
    Donut,
    /// Scatter.
    Scatter,
    /// Bubble (scatter + size).
    Bubble,
    /// Combo: columns plus a line on a secondary axis.
    Combo,
    /// Histogram of a single numeric series.
    Histogram,
}

impl ChartKind {
    /// Parse a config / command kind name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "line" => Self::Line,
            "column" | "col" => Self::Column,
            "bar" => Self::Bar,
            "column_stacked" | "col_stacked" => Self::ColumnStacked,
            "bar_stacked" => Self::BarStacked,
            "column_pct" | "col_pct" => Self::ColumnPct,
            "bar_pct" => Self::BarPct,
            "area" => Self::Area,
            "pie" => Self::Pie,
            "donut" | "doughnut" => Self::Donut,
            "scatter" => Self::Scatter,
            "bubble" => Self::Bubble,
            "combo" => Self::Combo,
            "histogram" => Self::Histogram,
            _ => return None,
        })
    }

    /// Config / command name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Column => "column",
            Self::Bar => "bar",
            Self::ColumnStacked => "column_stacked",
            Self::BarStacked => "bar_stacked",
            Self::ColumnPct => "column_pct",
            Self::BarPct => "bar_pct",
            Self::Area => "area",
            Self::Pie => "pie",
            Self::Donut => "donut",
            Self::Scatter => "scatter",
            Self::Bubble => "bubble",
            Self::Combo => "combo",
            Self::Histogram => "histogram",
        }
    }

    /// Whether this kind stacks series.
    #[must_use]
    pub const fn stacked(self) -> bool {
        matches!(
            self,
            Self::ColumnStacked | Self::BarStacked | Self::ColumnPct | Self::BarPct | Self::Area
        )
    }

    /// Whether this kind is a 100% stack.
    #[must_use]
    pub const fn percent(self) -> bool {
        matches!(self, Self::ColumnPct | Self::BarPct)
    }

    /// Horizontal bars rather than vertical columns.
    #[must_use]
    pub const fn horizontal(self) -> bool {
        matches!(self, Self::Bar | Self::BarStacked | Self::BarPct)
    }
}

/// Trendline kind (WP-25; error bars stay v1.x).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrendlineKind {
    /// Ordinary least squares.
    Linear,
    /// `y = a * e^(b x)` on positive y.
    Exponential,
    /// Trailing moving average.
    MovingAverage,
}

/// One trendline attached to a series.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trendline {
    /// Kind.
    pub kind: TrendlineKind,
    /// Window for [`TrendlineKind::MovingAverage`] (2–period default).
    #[serde(default = "trend_period")]
    pub period: u32,
}

fn trend_period() -> u32 {
    2
}

/// One data series.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Series {
    /// Legend name.
    pub name: String,
    /// Y (or value) range.
    pub values: RangeRef,
    /// Optional X / category range (scatter/bubble).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<RangeRef>,
    /// Optional bubble-size range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<RangeRef>,
    /// Optional `#rrggbb` override (file-authored; not retinted on theme swap).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Plot on the secondary value axis.
    #[serde(default)]
    pub secondary_axis: bool,
    /// Optional trendline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trendline: Option<Trendline>,
}

/// Axis chrome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Axis {
    /// Title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Draw gridlines.
    #[serde(default = "yes")]
    pub gridlines: bool,
}

fn yes() -> bool {
    true
}

impl Default for Axis {
    fn default() -> Self {
        Self {
            title: None,
            gridlines: true,
        }
    }
}

/// Legend placement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegendPos {
    /// Right of the plot.
    #[default]
    Right,
    /// Below the plot.
    Bottom,
    /// Hidden.
    None,
}

/// Two-cell drawing anchor in sheet cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChartAnchor {
    /// Inclusive start row.
    pub from_row: u32,
    /// Inclusive start column.
    pub from_col: u16,
    /// Inclusive end row.
    pub to_row: u32,
    /// Inclusive end column.
    pub to_col: u16,
}

impl Default for ChartAnchor {
    fn default() -> Self {
        Self {
            from_row: 1,
            from_col: 4,
            to_row: 16,
            to_col: 12,
        }
    }
}

/// One chart on a sheet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Chart {
    /// Stable id.
    pub id: ChartId,
    /// Kind.
    pub kind: ChartKind,
    /// Title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Category / X labels (ignored for pie when series names are used).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub categories: Option<RangeRef>,
    /// Series.
    pub series: Vec<Series>,
    /// Category axis.
    #[serde(default)]
    pub category_axis: Axis,
    /// Primary value axis.
    #[serde(default)]
    pub value_axis: Axis,
    /// Secondary value axis (combo).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary_axis: Option<Axis>,
    /// Legend.
    #[serde(default)]
    pub legend: LegendPos,
    /// Show data labels.
    #[serde(default)]
    pub data_labels: bool,
    /// Overlay position.
    #[serde(default)]
    pub anchor: ChartAnchor,
    /// Sheet the ranges default to.
    pub sheet: SheetId,
}

/// In-cell sparkline kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SparklineKind {
    /// Line.
    Line,
    /// Column.
    Column,
    /// Win/loss.
    WinLoss,
}

impl SparklineKind {
    /// Parse a name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "line" => Self::Line,
            "column" | "col" => Self::Column,
            "winloss" | "win_loss" => Self::WinLoss,
            _ => return None,
        })
    }
}

/// One sparkline (data range → display cell).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sparkline {
    /// Kind.
    pub kind: SparklineKind,
    /// Source values.
    pub data: RangeRef,
    /// Display cell row.
    pub row: u32,
    /// Display cell column.
    pub col: u16,
    /// Sheet.
    pub sheet: SheetId,
}

/// Palette and chrome used by the vector renderer (no conf dependency).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChartTheme {
    /// Plot background `#rrggbb`.
    pub background: String,
    /// Title / legend text.
    pub foreground: String,
    /// Axis line and tick labels.
    pub axis: String,
    /// Gridline.
    pub gridline: String,
    /// Series cycle.
    pub palette: [String; 8],
}

impl ChartTheme {
    /// Neutral fallback used by tests that do not load Omarchy.
    #[must_use]
    pub fn neutral() -> Self {
        Self {
            background: "#1a1b26".into(),
            foreground: "#c0caf5".into(),
            axis: "#a9b1d6".into(),
            gridline: "#3b4261".into(),
            palette: [
                "#7aa2f7".into(),
                "#9ece6a".into(),
                "#e0af68".into(),
                "#f7768e".into(),
                "#bb9af7".into(),
                "#7dcfff".into(),
                "#ff9e64".into(),
                "#c0caf5".into(),
            ],
        }
    }

    /// Series color, honouring an optional override.
    #[must_use]
    pub fn series_color<'a>(&'a self, index: usize, override_hex: Option<&'a str>) -> &'a str {
        override_hex.unwrap_or_else(|| {
            self.palette
                .get(index % self.palette.len())
                .map(String::as_str)
                .unwrap_or("#888888")
        })
    }
}
