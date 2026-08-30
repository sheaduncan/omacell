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
    if !goal.is_finite() || !tol.is_finite() || tol <= 0.0 {
        return Err(CoreError::new(
            "goalseek.args",
            "goal and tolerance must be finite and tolerance must be positive",
        ));
    }
    let max_iter = max_iter.clamp(1, 10_000);
    let original = wb.get(input.sheet, input.row, input.col)?.copied();
    let undo = wb.undo_log().is_enabled();
    wb.undo_log_mut().set_enabled(false);
    let result = goal_seek_inner(wb, engine, target, goal, input, max_iter, tol);
    wb.undo_log_mut().set_enabled(undo);
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let _ = wb.write_slot(input.sheet, input.row, input.col, original);
            engine.notify_edit(wb, input);
            engine.recalc_incremental(wb);
            return Err(error);
        }
    };
    // Record a single input-cell delta at the last trial (Excel leaves the value).
    wb.set_number(input.sheet, input.row, input.col, result.input)?;
    engine.notify_edit(wb, input);
    engine.recalc_incremental(wb);
    Ok(GoalSeekResult {
        output: read_number(wb, target).unwrap_or(result.output),
        ..result
    })
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
    // Prefer a nearby second point.
    let mut x1 = if x0.abs() > 1.0 { x0 * 1.01 } else { x0 + 1.0 };
    let mut f1 = residual(wb, engine, target, input, x1, goal)?;
    let mut lo = x0.min(x1);
    let mut hi = x0.max(x1);
    let mut flo = if x0 <= x1 { f0 } else { f1 };
    let mut fhi = if x0 <= x1 { f1 } else { f0 };
    let mut iters = 2u32;
    for _ in 2..max_iter {
        iters += 1;
        let x2 = if (f1 - f0).abs() > 1e-14 {
            x1 - f1 * (x1 - x0) / (f1 - f0)
        } else {
            (lo + hi) / 2.0
        };
        if !x2.is_finite() {
            break;
        }
        let f2 = residual(wb, engine, target, input, x2, goal)?;
        if f2.abs() <= tol {
            return Ok(GoalSeekResult {
                converged: true,
                input: x2,
                output: read_number(wb, target).unwrap_or(goal),
                iterations: iters,
            });
        }
        // Expand or shrink the bracket.
        if flo.signum() != f2.signum() {
            hi = x2;
            fhi = f2;
        } else if fhi.signum() != f2.signum() {
            lo = x2;
            flo = f2;
        } else {
            lo = lo.min(x2);
            hi = hi.max(x2);
        }
        x0 = x1;
        f0 = f1;
        x1 = x2;
        f1 = f2;
        // Bisection step if the bracket has opposite signs.
        if flo.signum() != fhi.signum() && (hi - lo).abs() > tol {
            let mid = (lo + hi) / 2.0;
            let fm = residual(wb, engine, target, input, mid, goal)?;
            iters += 1;
            if fm.abs() <= tol {
                return Ok(GoalSeekResult {
                    converged: true,
                    input: mid,
                    output: read_number(wb, target).unwrap_or(goal),
                    iterations: iters,
                });
            }
            if flo.signum() != fm.signum() {
                hi = mid;
                fhi = fm;
            } else {
                lo = mid;
                flo = fm;
            }
            x1 = mid;
            f1 = fm;
        }
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
    wb.set_number(input.sheet, input.row, input.col, x)?;
    engine.notify_edit(wb, input);
    engine.recalc_incremental(wb);
    let y = read_number(wb, target).ok_or_else(|| {
        CoreError::new("goalseek.target", "target cell is not numeric after recalc")
            .with_hint("Goal Seek needs a formula that evaluates to a number")
    })?;
    Ok(y - goal)
}

fn read_number(wb: &Workbook, cell: CellCoord) -> Option<f64> {
    match wb.get(cell.sheet, cell.row, cell.col).ok().flatten()?.value {
        Value::Number(n) if n.is_finite() => Some(n),
        Value::Bool(true) => Some(1.0),
        Value::Bool(false) => Some(0.0),
        _ => None,
    }
}
