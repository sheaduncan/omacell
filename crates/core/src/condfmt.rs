//! Conditional formatting engine and resolved overlay (F-6.5).

use serde::{Deserialize, Serialize};

use crate::addr::{RangeRef, SheetId};
use crate::style::Color;
use crate::value::Value;
use crate::workbook::Workbook;

/// Differential style applied by a rule.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CfDxf {
    /// Fill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Color>,
    /// Font colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<Color>,
}

/// Where an overlay colour came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlaySource {
    /// Explicit cell style from the file.
    File,
    /// Evaluated CF rule.
    Rule {
        /// Rule priority (1 = highest).
        priority: u32,
        /// Rule requested stop-if-true.
        stop: bool,
    },
}

/// Resolved paint for one cell (frontend cache; no UI-thread rule AST).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CfOverlay {
    /// Effective fill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Color>,
    /// Effective font colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<Color>,
    /// Provenance.
    pub source: OverlaySource,
}

/// CF operator for cell-value rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CfOp {
    /// Greater than.
    Greater,
    /// Less than.
    Less,
    /// Equal.
    Equal,
    /// Between.
    Between,
    /// Not between.
    NotBetween,
    /// Greater or equal.
    GreaterEq,
    /// Less or equal.
    LessEq,
    /// Not equal.
    NotEqual,
}

/// Rule kind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CfKind {
    /// Cell value compare.
    CellIs {
        /// Operator.
        op: CfOp,
        /// First formula/value.
        formula1: String,
        /// Second formula/value.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        formula2: Option<String>,
    },
    /// Contains text.
    ContainsText(String),
    /// Blanks.
    Blanks,
    /// Errors.
    Errors,
    /// Duplicate values in the range.
    Duplicate,
    /// Unique values.
    Unique,
    /// Top/bottom N or %.
    TopN {
        /// N.
        n: u32,
        /// Percent.
        percent: bool,
        /// Bottom.
        bottom: bool,
    },
    /// Above/below average.
    Average {
        /// Below.
        below: bool,
    },
    /// 2/3-color scale.
    ColorScale {
        /// Stops (2 or 3).
        colors: Vec<Color>,
    },
    /// Data bar.
    DataBar {
        /// Bar colour.
        color: Color,
        /// Gradient fill.
        gradient: bool,
    },
    /// Icon set (thresholds as percentiles 0–100).
    IconSet {
        /// Number of icons (3–5).
        icons: u8,
    },
    /// Formula is truthy (relative to the cell).
    Formula(String),
}

/// One conditional format rule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CondFormat {
    /// Applies to.
    pub range: RangeRef,
    /// Priority (1 wins).
    pub priority: u32,
    /// Stop if this rule matches.
    #[serde(default)]
    pub stop_if_true: bool,
    /// Kind.
    pub kind: CfKind,
    /// Differential style.
    #[serde(default)]
    pub dxf: CfDxf,
}

/// Overlay for one cell: file style, then CF rules by ascending priority.
#[must_use]
pub fn overlay_at(wb: &Workbook, sheet: SheetId, row: u32, col: u16) -> CfOverlay {
    let cache = RuleCache::new(wb, sheet);
    let slot = wb
        .sheet(sheet)
        .and_then(|s| s.store.get(row, col).ok().flatten());
    overlay_slot(wb, sheet, row, col, slot, &cache)
}

/// Overlays for a rectangle (row-major). Stats for top-N / average / scales
/// are computed once per rule.
pub fn overlay_range(wb: &Workbook, sheet: SheetId, range: RangeRef) -> Vec<(u32, u16, CfOverlay)> {
    let cache = RuleCache::new(wb, sheet);
    let (r0, c0, r1, c1) = norm(range);
    let mut out = Vec::with_capacity(
        ((r1.saturating_sub(r0) as usize).saturating_add(1))
            .saturating_mul((u32::from(c1.saturating_sub(c0)) as usize).saturating_add(1)),
    );
    let Some(s) = wb.sheet(sheet) else {
        return out;
    };
    for r in r0..=r1 {
        for c in c0..=c1 {
            let slot = s.store.get(r, c).ok().flatten();
            out.push((r, c, overlay_slot(wb, sheet, r, c, slot, &cache)));
        }
    }
    out
}

struct RuleCache<'a> {
    rules: Vec<&'a CondFormat>,
    nums: Vec<(u32, Vec<f64>)>,
    cell_is: Vec<(u32, CfOp, Option<f64>, Option<f64>)>,
}

fn cmp_cell_is(n: f64, op: CfOp, lo: Option<f64>, hi: Option<f64>) -> bool {
    match op {
        CfOp::Greater => lo.is_some_and(|x| n > x),
        CfOp::Less => lo.is_some_and(|x| n < x),
        CfOp::Equal => lo.is_some_and(|x| (n - x).abs() < 1e-12),
        CfOp::NotEqual => lo.is_none_or(|x| (n - x).abs() >= 1e-12),
        CfOp::GreaterEq => lo.is_some_and(|x| n >= x),
        CfOp::LessEq => lo.is_some_and(|x| n <= x),
        CfOp::Between => n >= lo.unwrap_or(n) && n <= hi.unwrap_or(n),
        CfOp::NotBetween => n < lo.unwrap_or(n) || n > hi.unwrap_or(n),
    }
}

impl<'a> RuleCache<'a> {
    fn new(wb: &'a Workbook, sheet: SheetId) -> Self {
        let Some(s) = wb.sheet(sheet) else {
            return Self {
                rules: Vec::new(),
                nums: Vec::new(),
                cell_is: Vec::new(),
            };
        };
        let mut rules: Vec<&CondFormat> = s.cond_formats.iter().collect();
        rules.sort_by_key(|r| r.priority);
        let mut nums = Vec::new();
        let mut cell_is = Vec::new();
        for rule in &rules {
            match &rule.kind {
                CfKind::CellIs {
                    op,
                    formula1,
                    formula2,
                } => {
                    cell_is.push((
                        rule.priority,
                        *op,
                        formula1.parse().ok(),
                        formula2.as_deref().and_then(|s| s.parse().ok()),
                    ));
                }
                CfKind::TopN { .. }
                | CfKind::Average { .. }
                | CfKind::ColorScale { .. }
                | CfKind::DataBar { .. }
                | CfKind::IconSet { .. }
                | CfKind::Duplicate
                | CfKind::Unique => {
                    nums.push((rule.priority, numbers_in(wb, sheet, rule.range)));
                }
                _ => {}
            }
        }
        Self {
            rules,
            nums,
            cell_is,
        }
    }

    fn nums(&self, priority: u32) -> &[f64] {
        self.nums
            .iter()
            .find(|(p, _)| *p == priority)
            .map(|(_, v)| v.as_slice())
            .unwrap_or(&[])
    }
}

fn overlay_slot(
    wb: &Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
    slot: Option<&crate::storage::CellSlot>,
    cache: &RuleCache<'_>,
) -> CfOverlay {
    let val = slot.map(|s| s.value).unwrap_or(Value::Empty);
    let file = file_style_slot(wb, slot);
    let mut out = file;
    let mut matched = false;
    for rule in &cache.rules {
        if !in_range(rule.range, row, col) {
            continue;
        }
        if matches_rule(wb, sheet, row, col, val, rule, cache) {
            if let Some(fill) = rule_fill(val, rule, cache) {
                out.fill = Some(fill);
            } else if let Some(fill) = rule.dxf.fill {
                out.fill = Some(fill);
            }
            if let Some(font) = rule.dxf.font {
                out.font = Some(font);
            }
            out.source = OverlaySource::Rule {
                priority: rule.priority,
                stop: rule.stop_if_true,
            };
            matched = true;
            if rule.stop_if_true {
                return out;
            }
        }
    }
    if matched { out } else { file }
}

fn rule_fill(val: Value, rule: &CondFormat, cache: &RuleCache<'_>) -> Option<Color> {
    let Value::Number(n) = val else {
        return None;
    };
    match &rule.kind {
        CfKind::ColorScale { colors } => {
            let nums = cache.nums(rule.priority);
            let min = nums.iter().copied().fold(f64::INFINITY, f64::min);
            let max = nums.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            color_scale_at(n, min, max, colors)
        }
        CfKind::DataBar { color, .. } => Some(*color),
        _ => None,
    }
}

fn file_style_slot(wb: &Workbook, slot: Option<&crate::storage::CellSlot>) -> CfOverlay {
    let mut fill = None;
    let mut font = None;
    if let Some(slot) = slot
        && let Some(style) = wb.intern().styles.get(slot.style)
    {
        fill = match style.fill {
            crate::style::Fill::Solid { fg } => Some(fg),
            crate::style::Fill::Pattern { fg, .. } => Some(fg),
            _ => None,
        };
        font = match style.font.color {
            Color::Auto => None,
            c => Some(c),
        };
    }
    CfOverlay {
        fill,
        font,
        source: OverlaySource::File,
    }
}

fn matches_rule(
    wb: &Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
    val: Value,
    rule: &CondFormat,
    cache: &RuleCache<'_>,
) -> bool {
    match &rule.kind {
        CfKind::CellIs {
            op,
            formula1,
            formula2,
        } => {
            let Value::Number(n) = val else {
                return false;
            };
            if let Some((_, cop, lo, hi)) = cache
                .cell_is
                .iter()
                .find(|(p, _, _, _)| *p == rule.priority)
            {
                return cmp_cell_is(n, *cop, *lo, *hi);
            }
            let lo = formula1.parse::<f64>().ok();
            let hi = formula2.as_deref().and_then(|s| s.parse().ok());
            cmp_cell_is(n, *op, lo, hi)
        }
        CfKind::ContainsText(needle) => display(wb, val)
            .to_lowercase()
            .contains(&needle.to_lowercase()),
        CfKind::Blanks => matches!(val, Value::Empty),
        CfKind::Errors => matches!(val, Value::Error(_)),
        CfKind::Duplicate => count_equal(wb, rule.range, sheet, &val) > 1,
        CfKind::Unique => count_equal(wb, rule.range, sheet, &val) == 1,
        CfKind::TopN { n, percent, bottom } => {
            top_n_cached(cache.nums(rule.priority), val, *n, *percent, *bottom)
        }
        CfKind::Average { below } => average_cached(cache.nums(rule.priority), val, *below),
        CfKind::ColorScale { .. } | CfKind::DataBar { .. } | CfKind::IconSet { .. } => {
            matches!(val, Value::Number(_))
        }
        CfKind::Formula(src) => eval_truthy_at(wb, sheet, row, col, src),
    }
}

/// Whether a formula (with or without `=`) is Excel-truthy in a vacuum.
#[must_use]
pub fn eval_truthy(wb: &Workbook, src: &str) -> bool {
    eval_truthy_at(wb, wb.active_sheet(), 0, 0, src)
}

fn eval_truthy_at(wb: &Workbook, sheet: SheetId, row: u32, col: u16, src: &str) -> bool {
    use crate::coerce::Scalar;
    use crate::eval::{FnRegistry, RuntimeValue, eval_formula};
    use crate::graph::CellCoord;
    use crate::spill::SpillTable;
    let text = if src.trim_start().starts_with('=') {
        src.to_string()
    } else {
        format!("={src}")
    };
    let Ok(parsed) = crate::formula::parse(&text) else {
        return false;
    };
    let registry = FnRegistry::new();
    let spill = SpillTable::new();
    let (value, _) = eval_formula(
        wb,
        &registry,
        &spill,
        CellCoord::new(sheet, row, col),
        &parsed.ast,
        0,
    );
    match value {
        RuntimeValue::Scalar(Scalar::Bool(b)) => b,
        RuntimeValue::Scalar(Scalar::Number(n)) => n != 0.0,
        RuntimeValue::Scalar(Scalar::Text(t)) => !t.is_empty(),
        RuntimeValue::Scalar(Scalar::Empty) => false,
        RuntimeValue::Scalar(Scalar::Error(_)) => false,
        _ => true,
    }
}

fn count_equal(wb: &Workbook, range: RangeRef, sheet: SheetId, val: &Value) -> u32 {
    let (r0, c0, r1, c1) = norm(range);
    let mut n = 0u32;
    for r in r0..=r1 {
        for c in c0..=c1 {
            if let Ok(Some(slot)) = wb.get(sheet, r, c)
                && values_eq(wb, slot.value, *val)
            {
                n += 1;
            }
        }
    }
    n
}

fn values_eq(wb: &Workbook, a: Value, b: Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => (x - y).abs() < 1e-12,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Text(i), Value::Text(j)) => {
            wb.intern().strings.get(i) == wb.intern().strings.get(j)
        }
        (Value::Empty, Value::Empty) => true,
        (Value::Error(x), Value::Error(y)) => x == y,
        _ => false,
    }
}

fn top_n_cached(nums: &[f64], val: Value, n: u32, percent: bool, bottom: bool) -> bool {
    let Value::Number(cur) = val else {
        return false;
    };
    if nums.is_empty() {
        return false;
    }
    let mut sorted = nums.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if !bottom {
        sorted.reverse();
    }
    let take = if percent {
        ((sorted.len() as u32 * n) / 100).max(1) as usize
    } else {
        (n as usize).min(sorted.len())
    };
    sorted.iter().take(take).any(|x| (*x - cur).abs() < 1e-12)
}

fn average_cached(nums: &[f64], val: Value, below: bool) -> bool {
    let Value::Number(cur) = val else {
        return false;
    };
    if nums.is_empty() {
        return false;
    }
    let avg = nums.iter().sum::<f64>() / nums.len() as f64;
    if below { cur < avg } else { cur > avg }
}

fn numbers_in(wb: &Workbook, sheet: SheetId, range: RangeRef) -> Vec<f64> {
    let (r0, c0, r1, c1) = norm(range);
    let mut nums = Vec::new();
    for r in r0..=r1 {
        for c in c0..=c1 {
            if let Ok(Some(slot)) = wb.get(sheet, r, c)
                && let Value::Number(n) = slot.value
            {
                nums.push(n);
            }
        }
    }
    nums
}

fn display(wb: &Workbook, v: Value) -> String {
    match v {
        Value::Number(n) => n.to_string(),
        Value::Bool(true) => "TRUE".into(),
        Value::Bool(false) => "FALSE".into(),
        Value::Text(id) => wb.intern().strings.get(id).unwrap_or("").to_string(),
        _ => String::new(),
    }
}

fn in_range(r: RangeRef, row: u32, col: u16) -> bool {
    let (r0, c0, r1, c1) = norm(r);
    row >= r0 && row <= r1 && col >= c0 && col <= c1
}

fn norm(r: RangeRef) -> (u32, u16, u32, u16) {
    (
        r.start.row.min(r.end.row),
        r.start.col.min(r.end.col),
        r.start.row.max(r.end.row),
        r.start.col.max(r.end.col),
    )
}

/// Interpolate a 2/3-color scale for `n` given the range min/max.
#[must_use]
pub fn color_scale_at(n: f64, min: f64, max: f64, colors: &[Color]) -> Option<Color> {
    if colors.is_empty() {
        return None;
    }
    if (max - min).abs() < 1e-12 {
        return colors.first().copied();
    }
    let t = ((n - min) / (max - min)).clamp(0.0, 1.0);
    if colors.len() == 1 {
        return colors.first().copied();
    }
    if colors.len() == 2 {
        return Some(lerp(colors[0], colors[1], t));
    }
    if t <= 0.5 {
        Some(lerp(colors[0], colors[1], t * 2.0))
    } else {
        Some(lerp(colors[1], colors[2], (t - 0.5) * 2.0))
    }
}

fn lerp(a: Color, b: Color, t: f64) -> Color {
    let Color::Rgb { argb: aa } = a else {
        return b;
    };
    let Color::Rgb { argb: bb } = b else {
        return a;
    };
    let mix = |s: u32, d: u32| -> u32 {
        let s = (s & 0xFF) as f64;
        let d = (d & 0xFF) as f64;
        (s + (d - s) * t).round() as u32
    };
    Color::Rgb {
        argb: (mix(aa >> 24, bb >> 24) << 24)
            | (mix(aa >> 16, bb >> 16) << 16)
            | (mix(aa >> 8, bb >> 8) << 8)
            | mix(aa, bb),
    }
}
