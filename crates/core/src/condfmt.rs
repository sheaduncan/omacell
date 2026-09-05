//! Conditional formatting engine and resolved overlay (F-6.5).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::addr::{RangeRef, SheetId};
use crate::error::CoreError;
use crate::eval::FnRegistry;
use crate::graph::CellCoord;
use crate::style::Color;
use crate::value::Value;
use crate::workbook::Workbook;

/// Differential style applied by a rule.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Data-bar or icon-set visual resolved for the cell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual: Option<CfVisual>,
    /// Provenance.
    pub source: OverlaySource,
}

/// Non-style conditional-format visual resolved for one cell.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CfVisual {
    /// Data-bar geometry. `fraction` and `axis` are normalized to `0..=1`.
    DataBar {
        /// Bar colour.
        color: Color,
        /// Gradient rather than solid fill.
        gradient: bool,
        /// Normalized value position within the rule range.
        fraction: f64,
        /// Normalized zero-axis position.
        axis: f64,
    },
    /// Icon-set member (zero is the lowest bucket).
    Icon {
        /// Size of the icon set.
        icons: u8,
        /// Selected member.
        index: u8,
    },
}

/// Maximum cells in one resolved conditional-format overlay request.
pub const MAX_CF_OVERLAY_CELLS: u64 = 1_000_000;

/// Pre-evaluated conditional-format overlays for a rectangular viewport.
///
/// Build this on a worker from an immutable workbook snapshot, then let a
/// frontend query [`Self::get`] without parsing or evaluating rule formulas on
/// its paint thread.
#[derive(Clone, Debug)]
pub struct ResolvedCfOverlay {
    min_row: u32,
    min_col: u16,
    rows: u32,
    cols: u32,
    cells: Vec<CfOverlay>,
}

impl ResolvedCfOverlay {
    /// Resolved cell overlay, or `None` outside the cached rectangle.
    #[must_use]
    pub fn get(&self, row: u32, col: u16) -> Option<CfOverlay> {
        let row_offset = row.checked_sub(self.min_row)?;
        let col_offset = u32::from(col.checked_sub(self.min_col)?);
        if row_offset >= self.rows || col_offset >= self.cols {
            return None;
        }
        let index = u64::from(row_offset)
            .checked_mul(u64::from(self.cols))?
            .checked_add(u64::from(col_offset))?;
        self.cells.get(usize::try_from(index).ok()?).copied()
    }

    /// Number of cached cells.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Whether no cells were cached.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }
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

/// Relative calendar period used by date conditional formatting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CfTimePeriod {
    /// Current calendar day.
    Today,
    /// Previous calendar day.
    Yesterday,
    /// Next calendar day.
    Tomorrow,
    /// Today and the preceding six days.
    Last7Days,
    /// Current Sunday-through-Saturday week.
    ThisWeek,
    /// Previous Sunday-through-Saturday week.
    LastWeek,
    /// Next Sunday-through-Saturday week.
    NextWeek,
    /// Current calendar month.
    ThisMonth,
    /// Previous calendar month.
    LastMonth,
    /// Next calendar month.
    NextMonth,
}

/// Rule kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Date falls in a relative calendar period.
    TimePeriod(CfTimePeriod),
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
    /// Icon set (thresholds as percentages of the value range, 0–100).
    IconSet {
        /// Number of icons (3–5).
        icons: u8,
    },
    /// Formula is truthy (relative to the cell).
    Formula(String),
}

/// One conditional format rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    let registry = FnRegistry::new();
    overlay_at_with_registry(wb, sheet, row, col, &registry)
}

/// Overlay for one cell using the application's registered worksheet functions.
#[must_use]
pub fn overlay_at_with_registry(
    wb: &Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
    registry: &FnRegistry,
) -> CfOverlay {
    let cache = RuleCache::new(wb, sheet);
    let slot = wb
        .sheet(sheet)
        .and_then(|s| s.store.get(row, col).ok().flatten());
    overlay_slot(wb, sheet, row, col, slot, &cache, registry)
}

/// Overlays for a rectangle (row-major). Stats for top-N / average / scales
/// are computed once per rule.
pub fn overlay_range(
    wb: &Workbook,
    sheet: SheetId,
    range: RangeRef,
) -> Result<Vec<(u32, u16, CfOverlay)>, CoreError> {
    let registry = FnRegistry::new();
    overlay_range_with_registry(wb, sheet, range, &registry)
}

/// Resolve row-major overlays using the application's registered functions.
pub fn overlay_range_with_registry(
    wb: &Workbook,
    sheet: SheetId,
    range: RangeRef,
    registry: &FnRegistry,
) -> Result<Vec<(u32, u16, CfOverlay)>, CoreError> {
    let resolved = resolve_overlay_with_registry(wb, sheet, range, registry)?;
    let (r0, c0, r1, c1) = norm(range);
    let mut out = Vec::new();
    out.try_reserve_exact(resolved.len()).map_err(|_| {
        CoreError::new(
            "condfmt.limit",
            "conditional-format result allocation failed",
        )
    })?;
    let mut index = 0usize;
    for row in r0..=r1 {
        for col in c0..=c1 {
            if let Some(overlay) = resolved.cells.get(index).copied() {
                out.push((row, col, overlay));
            }
            index += 1;
        }
    }
    Ok(out)
}

/// Resolve a bounded rectangular overlay cache for a frontend worker.
pub fn resolve_overlay(
    wb: &Workbook,
    sheet: SheetId,
    range: RangeRef,
) -> Result<ResolvedCfOverlay, CoreError> {
    let registry = FnRegistry::new();
    resolve_overlay_with_registry(wb, sheet, range, &registry)
}

/// Resolve a bounded overlay cache using the application's worksheet functions.
pub fn resolve_overlay_with_registry(
    wb: &Workbook,
    sheet: SheetId,
    range: RangeRef,
    registry: &FnRegistry,
) -> Result<ResolvedCfOverlay, CoreError> {
    let cache = RuleCache::new(wb, sheet);
    let (r0, c0, r1, c1) = norm(range);
    let rows = r1 - r0 + 1;
    let cols = u32::from(c1 - c0) + 1;
    let area = u64::from(rows) * u64::from(cols);
    if area > MAX_CF_OVERLAY_CELLS {
        return Err(CoreError::new(
            "condfmt.limit",
            format!(
                "conditional-format overlay has {area} cells; maximum is {MAX_CF_OVERLAY_CELLS}"
            ),
        ));
    }
    let capacity = usize::try_from(area)
        .map_err(|_| CoreError::new("condfmt.limit", "overlay size is not addressable"))?;
    let mut cells = Vec::new();
    cells.try_reserve_exact(capacity).map_err(|_| {
        CoreError::new(
            "condfmt.limit",
            "conditional-format cache allocation failed",
        )
    })?;
    let Some(s) = wb.sheet(sheet) else {
        return Err(CoreError::sheet_id(format!(
            "unknown sheet {}",
            sheet.index()
        )));
    };
    for r in r0..=r1 {
        for c in c0..=c1 {
            let slot = s.store.get(r, c).ok().flatten();
            cells.push(overlay_slot(wb, sheet, r, c, slot, &cache, registry));
        }
    }
    Ok(ResolvedCfOverlay {
        min_row: r0,
        min_col: c0,
        rows,
        cols,
        cells,
    })
}

struct RuleCache<'a> {
    rules: Vec<&'a CondFormat>,
    nums: Vec<(u32, Vec<f64>)>,
    counts: Vec<(u32, BTreeMap<String, u32>)>,
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
                counts: Vec::new(),
                cell_is: Vec::new(),
            };
        };
        let mut rules: Vec<&CondFormat> = s.cond_formats.iter().collect();
        rules.sort_by_key(|r| r.priority);
        let mut nums = Vec::new();
        let mut counts = Vec::new();
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
                | CfKind::IconSet { .. } => {
                    nums.push((rule.priority, numbers_in(wb, sheet, rule.range)));
                }
                CfKind::Duplicate | CfKind::Unique => {
                    counts.push((rule.priority, value_counts(wb, sheet, rule.range)));
                }
                _ => {}
            }
        }
        Self {
            rules,
            nums,
            counts,
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

    fn count(&self, priority: u32, wb: &Workbook, value: Value) -> u32 {
        let Some(key) = value_key(wb, value) else {
            return 0;
        };
        self.counts
            .iter()
            .find(|(candidate, _)| *candidate == priority)
            .and_then(|(_, counts)| counts.get(&key))
            .copied()
            .unwrap_or(0)
    }
}

fn overlay_slot(
    wb: &Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
    slot: Option<&crate::storage::CellSlot>,
    cache: &RuleCache<'_>,
    registry: &FnRegistry,
) -> CfOverlay {
    let val = slot.map(|s| s.value).unwrap_or(Value::Empty);
    let file = file_style_slot(wb, slot);
    let mut out = file;
    let mut matched = false;
    let mut rule_fill_applied = false;
    let mut rule_font_applied = false;
    let mut rule_visual_applied = false;
    for rule in &cache.rules {
        if !in_range(rule.range, row, col) {
            continue;
        }
        if matches_rule(
            wb,
            CellCoord::new(sheet, row, col),
            val,
            rule,
            cache,
            registry,
        ) {
            if !rule_fill_applied && let Some(fill) = rule_fill(val, rule, cache).or(rule.dxf.fill)
            {
                out.fill = Some(fill);
                rule_fill_applied = true;
            }
            if !rule_font_applied && let Some(font) = rule.dxf.font {
                out.font = Some(font);
                rule_font_applied = true;
            }
            if !rule_visual_applied && let Some(visual) = rule_visual(val, rule, cache) {
                out.visual = Some(visual);
                rule_visual_applied = true;
            }
            if !matched {
                out.source = OverlaySource::Rule {
                    priority: rule.priority,
                    stop: rule.stop_if_true,
                };
            }
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
            if colors.len() >= 3 {
                color_scale_at_midpoint(n, min, median(nums)?, max, colors)
            } else {
                color_scale_at(n, min, max, colors)
            }
        }
        CfKind::DataBar { .. } => None,
        _ => None,
    }
}

fn rule_visual(val: Value, rule: &CondFormat, cache: &RuleCache<'_>) -> Option<CfVisual> {
    let Value::Number(value) = val else {
        return None;
    };
    let nums = cache.nums(rule.priority);
    match &rule.kind {
        CfKind::DataBar { color, gradient } => {
            let min = nums.iter().copied().fold(f64::INFINITY, f64::min);
            let max = nums.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            if !min.is_finite() || !max.is_finite() {
                return None;
            }
            let span = max - min;
            let fraction = if span.abs() < 1e-12 {
                1.0
            } else {
                ((value - min) / span).clamp(0.0, 1.0)
            };
            let axis = if span.abs() < 1e-12 {
                if value < 0.0 { 1.0 } else { 0.0 }
            } else {
                ((0.0 - min) / span).clamp(0.0, 1.0)
            };
            Some(CfVisual::DataBar {
                color: *color,
                gradient: *gradient,
                fraction,
                axis,
            })
        }
        CfKind::IconSet { icons } => {
            let icons = (*icons).clamp(3, 5);
            let min = nums.iter().copied().fold(f64::INFINITY, f64::min);
            let max = nums.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            if !min.is_finite() || !max.is_finite() {
                return None;
            }
            let index = if (max - min).abs() < 1e-12 {
                icons - 1
            } else {
                (((value - min) / (max - min)).clamp(0.0, 1.0) * f64::from(icons)).floor() as u8
            };
            Some(CfVisual::Icon {
                icons,
                index: index.min(icons - 1),
            })
        }
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
        visual: None,
        source: OverlaySource::File,
    }
}

fn matches_rule(
    wb: &Workbook,
    at: CellCoord,
    val: Value,
    rule: &CondFormat,
    cache: &RuleCache<'_>,
    registry: &FnRegistry,
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
                && lo.is_some()
                && (!matches!(cop, CfOp::Between | CfOp::NotBetween) || hi.is_some())
            {
                return cmp_cell_is(n, *cop, *lo, *hi);
            }
            let (origin_row, origin_col, _, _) = norm(rule.range);
            let origin = CellCoord::new(at.sheet, origin_row, origin_col);
            let lo = eval_number_relative_with_registry(wb, at, origin, formula1, registry);
            let hi = formula2.as_deref().and_then(|formula| {
                eval_number_relative_with_registry(wb, at, origin, formula, registry)
            });
            cmp_cell_is(n, *op, lo, hi)
        }
        CfKind::ContainsText(needle) => display(wb, val)
            .to_lowercase()
            .contains(&needle.to_lowercase()),
        CfKind::Blanks => matches!(val, Value::Empty),
        CfKind::Errors => matches!(val, Value::Error(_)),
        CfKind::Duplicate => cache.count(rule.priority, wb, val) > 1,
        CfKind::Unique => cache.count(rule.priority, wb, val) == 1,
        CfKind::TopN { n, percent, bottom } => {
            top_n_cached(cache.nums(rule.priority), val, *n, *percent, *bottom)
        }
        CfKind::Average { below } => average_cached(cache.nums(rule.priority), val, *below),
        CfKind::TimePeriod(period) => matches_time_period(wb, val, *period),
        CfKind::ColorScale { .. } | CfKind::DataBar { .. } | CfKind::IconSet { .. } => {
            matches!(val, Value::Number(_))
        }
        CfKind::Formula(src) => {
            let (origin_row, origin_col, _, _) = norm(rule.range);
            eval_truthy_relative_with_registry(
                wb,
                at,
                CellCoord::new(at.sheet, origin_row, origin_col),
                src,
                registry,
            )
        }
    }
}

fn matches_time_period(wb: &Workbook, value: Value, period: CfTimePeriod) -> bool {
    let Value::Number(value) = value else {
        return false;
    };
    if !value.is_finite() {
        return false;
    }
    let day = value.floor() as i64;
    if crate::dates::serial_to_date(day, wb.settings().date_system).is_none() {
        return false;
    }
    let today = current_day_serial(wb.settings().date_system);
    match period {
        CfTimePeriod::Today => day == today,
        CfTimePeriod::Yesterday => day == today.saturating_sub(1),
        CfTimePeriod::Tomorrow => day == today.saturating_add(1),
        CfTimePeriod::Last7Days => (today.saturating_sub(6)..=today).contains(&day),
        CfTimePeriod::ThisWeek | CfTimePeriod::LastWeek | CfTimePeriod::NextWeek => {
            let weekday = crate::dates::weekday_sun0(today, wb.settings().date_system)
                .map(i64::from)
                .unwrap_or(0);
            let this_start = today.saturating_sub(weekday);
            let start = match period {
                CfTimePeriod::LastWeek => this_start.saturating_sub(7),
                CfTimePeriod::NextWeek => this_start.saturating_add(7),
                _ => this_start,
            };
            (start..=start.saturating_add(6)).contains(&day)
        }
        CfTimePeriod::ThisMonth | CfTimePeriod::LastMonth | CfTimePeriod::NextMonth => {
            let Some(date) = crate::dates::serial_to_date(day, wb.settings().date_system) else {
                return false;
            };
            let Some(today_date) = crate::dates::serial_to_date(today, wb.settings().date_system)
            else {
                return false;
            };
            let month_index = i64::from(date.year) * 12 + i64::from(date.month);
            let today_index = i64::from(today_date.year) * 12 + i64::from(today_date.month);
            let target = match period {
                CfTimePeriod::LastMonth => today_index - 1,
                CfTimePeriod::NextMonth => today_index + 1,
                _ => today_index,
            };
            month_index == target
        }
    }
}

fn current_day_serial(system: crate::dates::DateSystem) -> i64 {
    const UNIX_EPOCH_1900: i64 = 25_569;
    const UNIX_EPOCH_1904: i64 = 24_107;
    let unix_days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| (duration.as_secs() / 86_400) as i64)
        .unwrap_or(0);
    unix_days
        + match system {
            crate::dates::DateSystem::Excel1900 => UNIX_EPOCH_1900,
            crate::dates::DateSystem::Excel1904 => UNIX_EPOCH_1904,
        }
}

/// Whether a formula (with or without `=`) is Excel-truthy in a vacuum.
#[must_use]
pub fn eval_truthy(wb: &Workbook, src: &str) -> bool {
    let registry = FnRegistry::new();
    eval_truthy_at(wb, wb.active_sheet(), 0, 0, src, &registry)
}

fn eval_truthy_at(
    wb: &Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
    src: &str,
    registry: &FnRegistry,
) -> bool {
    use crate::coerce::Scalar;
    match eval_runtime_at(wb, sheet, row, col, src, registry) {
        Some(crate::eval::RuntimeValue::Scalar(Scalar::Bool(b))) => b,
        Some(crate::eval::RuntimeValue::Scalar(Scalar::Number(n))) => n != 0.0,
        Some(crate::eval::RuntimeValue::Scalar(Scalar::Text(t))) => !t.is_empty(),
        Some(crate::eval::RuntimeValue::Scalar(Scalar::Empty)) | None => false,
        Some(crate::eval::RuntimeValue::Scalar(Scalar::Error(_))) => false,
        _ => true,
    }
}

pub(crate) fn eval_truthy_relative_with_registry(
    wb: &Workbook,
    at: CellCoord,
    origin: CellCoord,
    src: &str,
    registry: &FnRegistry,
) -> bool {
    let adjusted = adjust_relative_formula(at.row, at.col, origin.row, origin.col, src);
    eval_truthy_at(wb, at.sheet, at.row, at.col, &adjusted, registry)
}

pub(crate) fn eval_number_relative_with_registry(
    wb: &Workbook,
    at: CellCoord,
    origin: CellCoord,
    src: &str,
    registry: &FnRegistry,
) -> Option<f64> {
    let adjusted = adjust_relative_formula(at.row, at.col, origin.row, origin.col, src);
    match eval_runtime_at(wb, at.sheet, at.row, at.col, &adjusted, registry)? {
        crate::eval::RuntimeValue::Scalar(crate::coerce::Scalar::Number(value)) => Some(value),
        _ => None,
    }
}

fn adjust_relative_formula(
    row: u32,
    col: u16,
    origin_row: u32,
    origin_col: u16,
    src: &str,
) -> String {
    let formula = if src.trim_start().starts_with('=') {
        src.to_string()
    } else {
        format!("={src}")
    };
    let drow = i32::try_from(row)
        .ok()
        .zip(i32::try_from(origin_row).ok())
        .map(|(target, origin)| target - origin)
        .unwrap_or(0);
    let dcol = i32::from(col) - i32::from(origin_col);
    crate::formula::rewrite_print(&formula, &crate::formula::RewriteOp::Copy { drow, dcol })
        .unwrap_or(formula)
}

fn eval_runtime_at(
    wb: &Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
    src: &str,
    registry: &FnRegistry,
) -> Option<crate::eval::RuntimeValue> {
    use crate::eval::eval_formula;
    use crate::graph::CellCoord;
    use crate::spill::SpillTable;
    let text = if src.trim_start().starts_with('=') {
        src.to_string()
    } else {
        format!("={src}")
    };
    let parsed = crate::formula::parse(&text).ok()?;
    let spill = SpillTable::new();
    Some(
        eval_formula(
            wb,
            registry,
            &spill,
            CellCoord::new(sheet, row, col),
            &parsed.ast,
            0,
        )
        .0,
    )
}

fn value_counts(wb: &Workbook, sheet: SheetId, range: RangeRef) -> BTreeMap<String, u32> {
    let (r0, c0, r1, c1) = norm(range);
    let mut counts = BTreeMap::new();
    let Some(sheet) = wb.sheet(sheet) else {
        return counts;
    };
    for (_, _, slot) in sheet.store.iter_region(r0, c0, r1, c1) {
        if let Some(key) = value_key(wb, slot.value) {
            let count = counts.entry(key).or_insert(0u32);
            *count = count.saturating_add(1);
        }
    }
    counts
}

fn value_key(wb: &Workbook, value: Value) -> Option<String> {
    match value {
        Value::Empty => None,
        Value::Number(number) => Some(format!(
            "n:{:016x}",
            if number == 0.0 {
                0.0f64.to_bits()
            } else {
                number.to_bits()
            }
        )),
        Value::Bool(value) => Some(format!("b:{value}")),
        Value::Text(id) => Some(format!(
            "t:{}",
            wb.intern()
                .strings
                .get(id)
                .unwrap_or_default()
                .to_lowercase()
        )),
        Value::Error(error) => Some(format!("e:{}", error.as_str())),
        Value::Array(id) => Some(format!("a:{}", id.index())),
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
        (sorted.len() as u32 * n.min(100)).div_ceil(100).max(1) as usize
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
    let Some(sheet) = wb.sheet(sheet) else {
        return nums;
    };
    for (_, _, slot) in sheet.store.iter_region(r0, c0, r1, c1) {
        if let Value::Number(number) = slot.value {
            nums.push(number);
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
    color_scale_at_midpoint(n, min, min + (max - min) / 2.0, max, colors)
}

fn color_scale_at_midpoint(
    n: f64,
    min: f64,
    midpoint: f64,
    max: f64,
    colors: &[Color],
) -> Option<Color> {
    if colors.len() < 3 {
        return color_scale_at(n, min, max, colors);
    }
    if n <= midpoint {
        let span = midpoint - min;
        let t = if span.abs() < 1e-12 {
            1.0
        } else {
            ((n - min) / span).clamp(0.0, 1.0)
        };
        Some(lerp(colors[0], colors[1], t))
    } else {
        let span = max - midpoint;
        let t = if span.abs() < 1e-12 {
            1.0
        } else {
            ((n - midpoint) / span).clamp(0.0, 1.0)
        };
        Some(lerp(colors[1], colors[2], t))
    }
}

fn median(values: &[f64]) -> Option<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let upper = *sorted.get(sorted.len() / 2)?;
    if sorted.len().is_multiple_of(2) {
        let lower = *sorted.get(sorted.len() / 2 - 1)?;
        Some(lower / 2.0 + upper / 2.0)
    } else {
        Some(upper)
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
