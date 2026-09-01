//! Goal Seek (F-7.2). Data Tables and Scenario Manager are deferred.

use crate::error::CoreError;
use crate::graph::CellCoord;
use crate::recalc::RecalcEngine;
use crate::value::Value;
use crate::workbook::Workbook;

/// Result of a Goal Seek run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GoalSeekResult {
    /// Solver reached the tolerance.
    pub converged: bool,
    /// Final input cell value.
    pub input: f64,
    /// Final target cell value.
    pub output: f64,
    /// Iterations used.
    pub iterations: u32,
}

/// Default max iterations (Excel uses 100).
pub const DEFAULT_MAX_ITER: u32 = 100;
/// Default absolute tolerance on the target residual.
pub const DEFAULT_TOL: f64 = 1e-6;

/// Vary `input` until `target` evaluates to `goal`.
///
/// Secant iteration with a bisection fallback. The input cell is left at the
/// last trial value (Excel-like). Non-convergence returns `converged: false`
/// rather than an error.
pub fn goal_seek(
    wb: &mut Workbook,
    engine: &mut RecalcEngine,
    target: CellCoord,
    goal: f64,
    input: CellCoord,
    max_iter: u32,
    tol: f64,
) -> Result<GoalSeekResult, CoreError> {
    validate_goal_seek(wb, target, goal, input, max_iter, tol)?;
    let original = wb.get(input.sheet, input.row, input.col)?.copied();
    let undo = wb.undo_log().is_enabled();
    wb.undo_log_mut().set_enabled(false);
    let result = goal_seek_inner(wb, engine, target, goal, input, max_iter, tol);
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let _ = wb.write_slot(input.sheet, input.row, input.col, original);
            wb.undo_log_mut().set_enabled(undo);
            engine.notify_edit(wb, input);
            engine.recalc_incremental(wb);
            return Err(error);
        }
    };
    // Restore without recording the trials, then commit the final input as one exact delta.
    let restored = wb.write_slot(input.sheet, input.row, input.col, original);
    wb.undo_log_mut().set_enabled(undo);
    restored?;
    set_numeric_input(wb, input, result.input)?;
    engine.notify_edit(wb, input);
    engine.recalc_incremental(wb);
    Ok(GoalSeekResult {
        output: read_number(wb, target).unwrap_or(result.output),
        ..result
    })
}

/// Validate Goal Seek arguments without changing the workbook.
pub fn validate_goal_seek(
    wb: &Workbook,
    target: CellCoord,
    goal: f64,
    input: CellCoord,
    max_iter: u32,
    tol: f64,
) -> Result<(), CoreError> {
    if !goal.is_finite() || !tol.is_finite() || tol <= 0.0 {
        return Err(CoreError::new(
            "goalseek.args",
            "goal and tolerance must be finite and tolerance must be positive",
        ));
    }
    if !(1..=10_000).contains(&max_iter) {
        return Err(CoreError::new(
            "goalseek.args",
            "max_iter must be in 1..=10000",
        ));
    }
    if target == input {
        return Err(CoreError::new(
            "goalseek.args",
            "target and input must be different cells",
        ));
    }
    let target_slot = wb
        .get(target.sheet, target.row, target.col)?
        .ok_or_else(|| CoreError::new("goalseek.target", "target cell is empty"))?;
    if target_slot.formula.is_none() {
        return Err(
            CoreError::new("goalseek.target", "target cell must contain a formula")
                .with_hint("choose a formula cell whose result depends on the changing cell"),
        );
    }
    if let Some(slot) = wb.get(input.sheet, input.row, input.col)?
        && slot.formula.is_some()
    {
        return Err(CoreError::new(
            "goalseek.input",
            "input cell must be a value, not a formula",
        ));
    }
    if wb
        .sheet(input.sheet)
        .and_then(|sheet| sheet.array_formula_at(input.row, input.col))
        .is_some()
    {
        return Err(CoreError::new(
            "formula.array",
            "goal seek input cannot be part of a legacy array-formula range",
        )
        .with_hint("choose an input cell outside the fixed array-formula range"));
    }
    if wb
        .pivots()
        .iter()
        .any(|pivot| pivot.contains(input.sheet, input.row, input.col))
    {
        return Err(
            CoreError::new("pivot.readonly", "pivot output cells are read-only")
                .with_hint("choose an input cell outside the pivot output"),
        );
    }
    Ok(())
}

fn goal_seek_inner(
    wb: &mut Workbook,
    engine: &mut RecalcEngine,
    target: CellCoord,
    goal: f64,
    input: CellCoord,
    max_iter: u32,
    tol: f64,
) -> Result<GoalSeekResult, CoreError> {
    let mut x0 = read_number(wb, input).unwrap_or(0.0);
    let mut f0 = residual(wb, engine, target, input, x0, goal)?;
    if f0.abs() <= tol {
        return Ok(GoalSeekResult {
            converged: true,
            input: x0,
            output: read_number(wb, target).unwrap_or(goal),
            iterations: 1,
        });
    }
    if max_iter == 1 {
        return Ok(GoalSeekResult {
            converged: false,
            input: x0,
            output: read_number(wb, target).unwrap_or(f64::NAN),
            iterations: 1,
        });
    }
    // Prefer a nearby second point.
    let candidate = if x0.abs() > 1.0 { x0 * 1.01 } else { x0 + 1.0 };
    let mut x1 = if candidate.is_finite() {
        candidate
    } else {
        x0 * 0.99
    };
    let mut f1 = residual(wb, engine, target, input, x1, goal)?;
    let mut iters = 2u32;
    if f1.abs() <= tol {
        return Ok(GoalSeekResult {
            converged: true,
            input: x1,
            output: read_number(wb, target).unwrap_or(goal),
            iterations: iters,
        });
    }
    let mut bracket = opposite_sign(f0, f1).then_some(if x0 <= x1 {
        (x0, f0, x1, f1)
    } else {
        (x1, f1, x0, f0)
    });
    while iters < max_iter {
        let secant = if (f1 - f0).abs() > 1e-14 {
            Some(x1 - f1 * (x1 - x0) / (f1 - f0))
        } else {
            None
        };
        let x2 = match (secant.filter(|x| x.is_finite()), bracket) {
            (Some(x), Some((lo, _, hi, _))) if x > lo && x < hi => x,
            (_, Some((lo, _, hi, _))) => lo + (hi - lo) / 2.0,
            (Some(x), None) => x,
            (None, None) => {
                let step = (x1 - x0).abs().max(1.0);
                x1 + step
            }
        };
        if !x2.is_finite() {
            break;
        }
        let f2 = residual(wb, engine, target, input, x2, goal)?;
        iters += 1;
        if f2.abs() <= tol {
            return Ok(GoalSeekResult {
                converged: true,
                input: x2,
                output: read_number(wb, target).unwrap_or(goal),
                iterations: iters,
            });
        }
        bracket = update_bracket(bracket, x0, f0, x1, f1, x2, f2);
        x0 = x1;
        f0 = f1;
        x1 = x2;
        f1 = f2;
    }
    Ok(GoalSeekResult {
        converged: false,
        input: x1,
        output: read_number(wb, target).unwrap_or(f64::NAN),
        iterations: iters,
    })
}

fn residual(
    wb: &mut Workbook,
    engine: &mut RecalcEngine,
    target: CellCoord,
    input: CellCoord,
    x: f64,
    goal: f64,
) -> Result<f64, CoreError> {
    set_numeric_input(wb, input, x)?;
    engine.notify_edit(wb, input);
    engine.recalc_incremental(wb);
    let y = read_number(wb, target).ok_or_else(|| {
        CoreError::new("goalseek.target", "target cell is not numeric after recalc")
            .with_hint("Goal Seek needs a formula that evaluates to a number")
    })?;
    Ok(y - goal)
}

fn set_numeric_input(wb: &mut Workbook, input: CellCoord, value: f64) -> Result<(), CoreError> {
    let mut slot = wb
        .get(input.sheet, input.row, input.col)?
        .copied()
        .unwrap_or_else(crate::storage::CellSlot::empty);
    slot.value = Value::Number(value);
    slot.formula = None;
    wb.set_slot(input.sheet, input.row, input.col, slot)?;
    Ok(())
}

fn opposite_sign(a: f64, b: f64) -> bool {
    (a.is_sign_negative() && b.is_sign_positive()) || (a.is_sign_positive() && b.is_sign_negative())
}

#[allow(clippy::too_many_arguments)]
fn update_bracket(
    current: Option<(f64, f64, f64, f64)>,
    x0: f64,
    f0: f64,
    x1: f64,
    f1: f64,
    x2: f64,
    f2: f64,
) -> Option<(f64, f64, f64, f64)> {
    if let Some((lo, flo, hi, fhi)) = current {
        if opposite_sign(flo, f2) {
            return Some(order_pair(lo, flo, x2, f2));
        }
        if opposite_sign(fhi, f2) {
            return Some(order_pair(hi, fhi, x2, f2));
        }
    }
    if opposite_sign(f1, f2) {
        Some(order_pair(x1, f1, x2, f2))
    } else if opposite_sign(f0, f2) {
        Some(order_pair(x0, f0, x2, f2))
    } else {
        None
    }
}

fn order_pair(xa: f64, fa: f64, xb: f64, fb: f64) -> (f64, f64, f64, f64) {
    if xa <= xb {
        (xa, fa, xb, fb)
    } else {
        (xb, fb, xa, fa)
    }
}

fn read_number(wb: &Workbook, cell: CellCoord) -> Option<f64> {
    match wb.get(cell.sheet, cell.row, cell.col).ok().flatten()?.value {
        Value::Number(n) if n.is_finite() => Some(n),
        Value::Bool(true) => Some(1.0),
        Value::Bool(false) => Some(0.0),
        _ => None,
    }
}
