//! Tree-walking formula evaluator (F-3.3–F-3.5).

mod ops;
mod registry;

pub use registry::{ArrayLift, FnBody, FnDef, FnRegistry};

use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::addr::{CellRef, RangeRef, SheetId, SheetSpec, col_to_letters};
use crate::coerce::Scalar;
use crate::error::ErrorKind;
use crate::formula::{
    BinOp, Callee, Expr, ExprKind, Formula, PostfixOp, PrefixOp, StructuredRef, TableColumns,
    TableItem, parse,
};
use crate::graph::CellCoord;
use crate::intern::Interners;
use crate::lambda::{self, Lambda};
use crate::limits::{MAX_COLS, MAX_ROWS};
use crate::locale::LocaleId;
use crate::names::{NameReferent, NameScope};
use crate::recalc::{AsyncRequest, AsyncState, ContentHash};
use crate::spill::SpillTable;
use crate::tables::Table;
use crate::value::Value;
use crate::workbook::Workbook;

/// Maximum nested LAMBDA / call depth before `#NUM!`.
const MAX_CALL_DEPTH: u32 = 512;

/// Shape of a runtime array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    /// Rows.
    pub rows: u32,
    /// Columns.
    pub cols: u32,
}

/// A range / union / 3-D reference produced during evaluation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reference {
    /// A rectangle on one sheet.
    Range {
        /// Sheet.
        sheet: SheetId,
        /// Inclusive start row.
        start_row: u32,
        /// Inclusive start column.
        start_col: u16,
        /// Inclusive end row.
        end_row: u32,
        /// Inclusive end column.
        end_col: u16,
    },
    /// Union of references (`,`).
    Union(Vec<Reference>),
    /// Same rectangle on several sheets.
    ThreeD {
        /// Sheets in workbook order.
        sheets: Vec<SheetId>,
        /// Inclusive start row.
        start_row: u32,
        /// Inclusive start column.
        start_col: u16,
        /// Inclusive end row.
        end_row: u32,
        /// Inclusive end column.
        end_col: u16,
    },
}

impl Reference {
    fn cell(sheet: SheetId, row: u32, col: u16) -> Self {
        Self::Range {
            sheet,
            start_row: row,
            start_col: col,
            end_row: row,
            end_col: col,
        }
    }

    fn is_single_cell(&self) -> bool {
        match self {
            Self::Range {
                start_row,
                start_col,
                end_row,
                end_col,
                ..
            } => start_row == end_row && start_col == end_col,
            Self::ThreeD {
                sheets,
                start_row,
                start_col,
                end_row,
                end_col,
            } => sheets.len() <= 1 && start_row == end_row && start_col == end_col,
            Self::Union(v) => v.len() == 1 && v[0].is_single_cell(),
        }
    }
}

/// Array payload (row-major scalars).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeArray {
    /// Rows (≥ 1).
    pub rows: u32,
    /// Columns (≥ 1).
    pub cols: u32,
    /// Row-major values.
    pub values: Arc<[Scalar]>,
}

impl RuntimeArray {
    /// Checked constructor. Rejects zero, out-of-grid, overflowing, or
    /// payload-mismatched shapes **before** storing the values.
    pub fn try_new(rows: u32, cols: u32, values: Vec<Scalar>) -> Result<Self, ErrorKind> {
        if rows == 0 || cols == 0 {
            return Err(ErrorKind::Num);
        }
        if rows > MAX_ROWS || cols > u32::from(MAX_COLS) {
            return Err(ErrorKind::Num);
        }
        let len = rows.checked_mul(cols).ok_or(ErrorKind::Num)?;
        if values.len() != len as usize {
            return Err(ErrorKind::Value);
        }
        Ok(Self {
            rows,
            cols,
            values: values.into(),
        })
    }
}

/// Runtime value used during evaluation. Commit maps this to interned [`Value`].
#[derive(Clone, Debug)]
pub enum RuntimeValue {
    /// Scalar.
    Scalar(Scalar),
    /// Dynamic array (may spill).
    Array(Arc<RuntimeArray>),
    /// Uncalled lambda (stored as `#CALC!`).
    Lambda(Arc<Lambda>),
    /// Unevaluated reference (range/union/3-D).
    Ref(Reference),
}

impl PartialEq for RuntimeValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Scalar(a), Self::Scalar(b)) => a == b,
            (Self::Array(a), Self::Array(b)) => a == b,
            (Self::Lambda(a), Self::Lambda(b)) => a == b,
            (Self::Ref(a), Self::Ref(b)) => a == b,
            _ => false,
        }
    }
}

impl RuntimeValue {
    /// Excel error value.
    #[must_use]
    pub fn error(e: ErrorKind) -> Self {
        Self::Scalar(Scalar::Error(e))
    }

    /// Error kind if this is a scalar error (not nested in an array).
    #[must_use]
    pub fn error_kind(&self) -> Option<ErrorKind> {
        match self {
            Self::Scalar(Scalar::Error(e)) => Some(*e),
            _ => None,
        }
    }

    /// Build an array, collapsing 1×1 to a scalar. Invalid shapes become errors.
    #[must_use]
    pub fn array(rows: u32, cols: u32, values: Vec<Scalar>) -> Self {
        match RuntimeArray::try_new(rows, cols, values) {
            Ok(array) if array.rows == 1 && array.cols == 1 => {
                Self::Scalar(array.values.first().cloned().unwrap_or(Scalar::Empty))
            }
            Ok(array) => Self::Array(Arc::new(array)),
            Err(error) => Self::error(error),
        }
    }

    /// Checked array construction (same rules as [`RuntimeArray::try_new`]).
    pub fn try_array(rows: u32, cols: u32, values: Vec<Scalar>) -> Result<Self, ErrorKind> {
        let array = RuntimeArray::try_new(rows, cols, values)?;
        if array.rows == 1 && array.cols == 1 {
            Ok(Self::Scalar(
                array.values.first().cloned().unwrap_or(Scalar::Empty),
            ))
        } else {
            Ok(Self::Array(Arc::new(array)))
        }
    }

    /// Map a stored cell value into a runtime scalar (arrays stay arrays).
    #[must_use]
    pub fn from_stored(value: Value, intern: &Interners) -> Self {
        match value {
            Value::Empty => Self::Scalar(Scalar::Empty),
            Value::Number(n) => Self::Scalar(Scalar::Number(n)),
            Value::Bool(b) => Self::Scalar(Scalar::Bool(b)),
            Value::Text(id) => match intern.strings.get(id) {
                Some(s) => Self::Scalar(Scalar::Text(Arc::from(s))),
                None => Self::error(ErrorKind::Value),
            },
            Value::Error(e) => Self::error(e),
            Value::Array(id) => match intern.arrays.get(id) {
                Some(p) => {
                    let values: Vec<Scalar> = p
                        .values
                        .iter()
                        .map(|v| match Self::from_stored(*v, intern) {
                            Self::Scalar(s) => s,
                            _ => Scalar::Empty,
                        })
                        .collect();
                    Self::array(p.shape.rows, p.shape.cols, values)
                }
                None => Self::error(ErrorKind::Value),
            },
        }
    }
}

/// One evaluated (or omitted) function argument.
#[derive(Clone, Debug)]
pub struct ArgVal {
    /// True when the call site omitted this argument (`INDEX(a,,2)`).
    pub omitted: bool,
    /// Value (empty when omitted).
    pub value: RuntimeValue,
}

/// Pass-stable calculation environment, sampled once before parallel eval.
///
/// Not stored on frozen [`crate::workbook::WorkbookSettings`].
#[derive(Clone, Copy, Debug)]
pub struct PassEnv {
    /// Excel 1900 date serial (including time fraction) for `NOW`/`TODAY`.
    pub clock: f64,
    /// Locale for `TEXT` / `VALUE` / `DATEVALUE` (WP-05b).
    pub locale: LocaleId,
    /// Seed from which volatile random functions derive per-cell values.
    pub random_nonce: u64,
}

impl Default for PassEnv {
    fn default() -> Self {
        Self {
            clock: 0.0,
            locale: LocaleId::EN_US,
            random_nonce: 0,
        }
    }
}

impl PassEnv {
    /// Unit-interval random for `function` at `index` in `cell` on `pass`.
    #[must_use]
    pub fn random_unit(self, cell: CellCoord, pass: u32, function: &str, index: u32) -> f64 {
        let mut h = self.random_nonce;
        h ^= u64::from(pass).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h ^= u64::from(cell.sheet.index()).wrapping_shl(32);
        h ^= u64::from(cell.row);
        h ^= u64::from(cell.col).wrapping_shl(16);
        h ^= u64::from(index).wrapping_shl(8);
        for byte in function.as_bytes() {
            h = h.wrapping_mul(0x0100_0000_01B3) ^ u64::from(*byte);
        }
        let mixed = splitmix64(h);
        (mixed >> 11) as f64 / ((1u64 << 53) as f64)
    }
}

fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[derive(Clone, Debug, Default)]
struct ScopeFrame {
    binds: Vec<(String, RuntimeValue)>,
    omitted: Vec<String>,
}

/// Evaluation context for one cell in one recalc pass.
pub struct EvalCtx<'a> {
    wb: &'a Workbook,
    registry: &'a FnRegistry,
    spill: &'a SpillTable,
    /// Formula cell being evaluated.
    pub cell: CellCoord,
    depth: u32,
    frames: Vec<ScopeFrame>,
    pass: u32,
    env: PassEnv,
    pending_async: bool,
    stale: bool,
    async_hint: Option<String>,
    resolved_dynamic: Vec<Reference>,
    async_provider: Option<&'a dyn crate::recalc::AsyncNodeProvider>,
}

impl<'a> EvalCtx<'a> {
    /// Build a context for evaluating `cell` on `pass`.
    pub fn new(
        wb: &'a Workbook,
        registry: &'a FnRegistry,
        spill: &'a SpillTable,
        cell: CellCoord,
        pass: u32,
    ) -> Self {
        Self {
            wb,
            registry,
            spill,
            cell,
            depth: 0,
            frames: Vec::new(),
            pass,
            env: PassEnv::default(),
            pending_async: false,
            stale: false,
            async_hint: None,
            resolved_dynamic: Vec::new(),
            async_provider: None,
        }
    }

    /// Attach the provider used by async function calls during this evaluation.
    #[must_use]
    pub fn with_async_provider(
        mut self,
        provider: Option<&'a dyn crate::recalc::AsyncNodeProvider>,
    ) -> Self {
        self.async_provider = provider;
        self
    }

    /// Attach the pass-stable clock / locale / random environment.
    #[must_use]
    pub fn with_pass_env(mut self, env: PassEnv) -> Self {
        self.env = env;
        self
    }

    /// Workbook (read-only during eval).
    #[must_use]
    pub fn workbook(&self) -> &Workbook {
        self.wb
    }

    /// Current recalc pass counter (volatile functions).
    #[must_use]
    pub fn pass(&self) -> u32 {
        self.pass
    }

    /// Formula cell coordinate.
    #[must_use]
    pub fn coord(&self) -> CellCoord {
        self.cell
    }

    /// Pass-stable environment for this evaluation.
    #[must_use]
    pub fn pass_env(&self) -> PassEnv {
        self.env
    }

    /// Injected / sampled `NOW` serial (date + time fraction).
    #[must_use]
    pub fn clock(&self) -> f64 {
        self.env.clock
    }

    /// Integer date serial for `TODAY`.
    #[must_use]
    pub fn today(&self) -> f64 {
        self.env.clock.trunc()
    }

    /// Locale for text/date conversion functions.
    #[must_use]
    pub fn locale(&self) -> LocaleId {
        self.env.locale
    }

    /// Deterministic unit random for this cell, function, and array index.
    #[must_use]
    pub fn random_unit(&self, function: &str, index: u32) -> f64 {
        self.env.random_unit(self.cell, self.pass, function, index)
    }

    pub(crate) fn take_flags(&mut self) -> (bool, bool, Option<String>, Vec<Reference>) {
        (
            self.pending_async,
            self.stale,
            self.async_hint.take(),
            std::mem::take(&mut self.resolved_dynamic),
        )
    }

    pub(crate) fn push_frame(&mut self) {
        self.frames.push(ScopeFrame::default());
    }

    pub(crate) fn pop_frame(&mut self) {
        self.frames.pop();
    }

    pub(crate) fn bind(&mut self, name: String, val: RuntimeValue) {
        if let Some(f) = self.frames.last_mut() {
            f.binds.push((name, val));
        }
    }

    pub(crate) fn bind_omitted(&mut self, name: String) {
        if let Some(f) = self.frames.last_mut() {
            f.omitted.push(name.clone());
            f.binds.push((name, RuntimeValue::Scalar(Scalar::Empty)));
        }
    }

    pub(crate) fn snapshot_scope(&self) -> Arc<[(String, RuntimeValue)]> {
        let mut all = Vec::new();
        for f in &self.frames {
            all.extend(f.binds.iter().cloned());
        }
        all.into()
    }

    pub(crate) fn is_omitted(&self, name: &str) -> bool {
        for f in self.frames.iter().rev() {
            if f.omitted.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                return true;
            }
            if f.binds.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
                return false;
            }
        }
        false
    }

    pub(crate) fn is_omitted_expr(&self, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Name { name, .. } => self.is_omitted(name),
            _ => false,
        }
    }

    pub(crate) fn enter_call(&mut self) -> Result<(), ErrorKind> {
        self.depth += 1;
        if self.depth > MAX_CALL_DEPTH {
            self.depth -= 1;
            Err(ErrorKind::Num)
        } else {
            Ok(())
        }
    }

    pub(crate) fn leave_call(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn scope_lookup(&self, name: &str) -> Option<RuntimeValue> {
        for f in self.frames.iter().rev() {
            for (n, v) in f.binds.iter().rev() {
                if n.eq_ignore_ascii_case(name) {
                    return Some(v.clone());
                }
            }
        }
        None
    }

    /// Read a stored cell as a runtime scalar (spill origin → top-left).
    #[must_use]
    pub fn read_cell(&self, sheet: SheetId, row: u32, col: u16) -> Scalar {
        let Ok(Some(slot)) = self.wb.get(sheet, row, col) else {
            return Scalar::Empty;
        };
        match RuntimeValue::from_stored(slot.value, self.wb.intern()) {
            RuntimeValue::Scalar(s) => s,
            RuntimeValue::Array(a) => a.values.first().cloned().unwrap_or(Scalar::Empty),
            RuntimeValue::Lambda(_) => Scalar::Error(ErrorKind::Calc),
            RuntimeValue::Ref(_) => Scalar::Empty,
        }
    }

    /// Walk every cell in a reference (row-major, sheets in order).
    pub fn for_each_cell(&self, r: &Reference, f: &mut impl FnMut(Scalar)) {
        match r {
            Reference::Range {
                sheet,
                start_row,
                start_col,
                end_row,
                end_col,
            } => {
                let r1 = (*start_row).min(*end_row);
                let r2 = (*start_row).max(*end_row);
                let c1 = (*start_col).min(*end_col);
                let c2 = (*start_col).max(*end_col);
                for row in r1..=r2 {
                    for col in c1..=c2 {
                        f(self.read_cell(*sheet, row, col));
                    }
                }
            }
            Reference::Union(parts) => {
                for p in parts {
                    self.for_each_cell(p, f);
                }
            }
            Reference::ThreeD {
                sheets,
                start_row,
                start_col,
                end_row,
                end_col,
            } => {
                for sheet in sheets {
                    self.for_each_cell(
                        &Reference::Range {
                            sheet: *sheet,
                            start_row: *start_row,
                            start_col: *start_col,
                            end_row: *end_row,
                            end_col: *end_col,
                        },
                        f,
                    );
                }
            }
        }
    }

    /// Materialize a reference into a scalar or array.
    #[must_use]
    pub fn materialize(&self, v: RuntimeValue) -> RuntimeValue {
        match v {
            RuntimeValue::Ref(r) => materialize_ref(self, &r),
            other => other,
        }
    }

    /// Mark that this evaluation issued an async request that is still pending.
    pub fn mark_pending(&mut self) {
        self.pending_async = true;
        self.stale = true;
    }

    /// Record a provider failure hint (A-3.6).
    pub fn fail_hint(&mut self, hint: impl Into<String>) {
        self.async_hint = Some(hint.into());
        self.stale = true;
    }

    /// Record a range resolved by a dynamic function (`INDIRECT` / `OFFSET`).
    pub fn record_dynamic_ref(&mut self, r: Reference) {
        self.resolved_dynamic.push(r);
    }
}

fn materialize_ref(ctx: &EvalCtx<'_>, r: &Reference) -> RuntimeValue {
    match r {
        Reference::Range {
            sheet,
            start_row,
            start_col,
            end_row,
            end_col,
        } => {
            let r1 = (*start_row).min(*end_row);
            let r2 = (*start_row).max(*end_row);
            let c1 = (*start_col).min(*end_col);
            let c2 = (*start_col).max(*end_col);
            let rows = r2.saturating_sub(r1) + 1;
            let cols = u32::from(c2.saturating_sub(c1) + 1);
            if rows == 1 && cols == 1 {
                return RuntimeValue::Scalar(ctx.read_cell(*sheet, r1, c1));
            }
            let mut values = Vec::with_capacity((rows as usize) * (cols as usize));
            for row in r1..=r2 {
                for col in c1..=c2 {
                    values.push(ctx.read_cell(*sheet, row, col));
                }
            }
            RuntimeValue::array(rows, cols, values)
        }
        Reference::Union(parts) => {
            if parts.len() == 1 {
                return materialize_ref(ctx, &parts[0]);
            }
            RuntimeValue::error(ErrorKind::Value)
        }
        Reference::ThreeD {
            sheets,
            start_row,
            start_col,
            end_row,
            end_col,
        } => {
            if sheets.is_empty() {
                return RuntimeValue::error(ErrorKind::Ref);
            }
            let mut rows_out: Vec<Vec<Scalar>> = Vec::new();
            for sheet in sheets {
                match materialize_ref(
                    ctx,
                    &Reference::Range {
                        sheet: *sheet,
                        start_row: *start_row,
                        start_col: *start_col,
                        end_row: *end_row,
                        end_col: *end_col,
                    },
                ) {
                    RuntimeValue::Scalar(s) => rows_out.push(vec![s]),
                    RuntimeValue::Array(a) => {
                        let cols = a.cols as usize;
                        for chunk in a.values.chunks(cols) {
                            rows_out.push(chunk.to_vec());
                        }
                    }
                    other => {
                        if let Some(e) = other.error_kind() {
                            return RuntimeValue::error(e);
                        }
                    }
                }
            }
            let cols = rows_out.first().map(|r| r.len() as u32).unwrap_or(1);
            let rows = rows_out.len() as u32;
            let values: Vec<Scalar> = rows_out.into_iter().flatten().collect();
            RuntimeValue::array(rows, cols.max(1), values)
        }
    }
}

/// Evaluate a parsed expression in `ctx`.
pub fn eval_expr(ctx: &mut EvalCtx<'_>, expr: &Expr) -> RuntimeValue {
    if ctx.depth > MAX_CALL_DEPTH {
        return RuntimeValue::error(ErrorKind::Num);
    }
    match &expr.kind {
        ExprKind::Number(n) => RuntimeValue::Scalar(Scalar::Number(*n)),
        ExprKind::String(s) => RuntimeValue::Scalar(Scalar::Text(Arc::from(s.as_str()))),
        ExprKind::Bool(b) => RuntimeValue::Scalar(Scalar::Bool(*b)),
        ExprKind::Error(e) => RuntimeValue::error(*e),
        ExprKind::Array(rows) => eval_array_lit(ctx, rows),
        ExprKind::Cell { sheet, cell } => eval_cell_ref(ctx, sheet.as_ref(), *cell),
        ExprKind::Range { sheet, range } => eval_range_ref(ctx, sheet.as_ref(), *range),
        ExprKind::ThreeD { sheets, inner } => eval_threed(ctx, sheets, inner),
        ExprKind::Name { sheet, name } => eval_name(ctx, sheet.as_ref(), name),
        ExprKind::Structured(sr) => eval_structured(ctx, sr),
        ExprKind::External { .. } => RuntimeValue::error(ErrorKind::Ref),
        ExprKind::Prefix { op, expr } => eval_prefix(ctx, *op, expr),
        ExprKind::Postfix { expr, op } => eval_postfix(ctx, expr, *op),
        ExprKind::Binary { op, left, right } => eval_binary(ctx, *op, left, right),
        ExprKind::Paren(inner) => eval_expr(ctx, inner),
        ExprKind::Call { callee, args } => eval_call(ctx, callee, args),
    }
}

fn eval_array_lit(ctx: &mut EvalCtx<'_>, rows: &[Vec<Expr>]) -> RuntimeValue {
    if rows.is_empty() {
        return RuntimeValue::error(ErrorKind::Value);
    }
    let nrows = rows.len() as u32;
    let ncols = rows.iter().map(|r| r.len()).max().unwrap_or(0) as u32;
    if ncols == 0 {
        return RuntimeValue::error(ErrorKind::Value);
    }
    let mut values = Vec::with_capacity((nrows as usize) * (ncols as usize));
    for row in rows {
        for c in 0..ncols as usize {
            if let Some(e) = row.get(c) {
                match eval_expr(ctx, e) {
                    RuntimeValue::Scalar(s) => values.push(s),
                    RuntimeValue::Array(_) | RuntimeValue::Ref(_) | RuntimeValue::Lambda(_) => {
                        values.push(Scalar::Error(ErrorKind::Value));
                    }
                }
            } else {
                values.push(Scalar::Empty);
            }
        }
    }
    RuntimeValue::array(nrows, ncols, values)
}

fn resolve_sheet(
    ctx: &EvalCtx<'_>,
    spec: Option<&SheetSpec>,
    default: SheetId,
) -> Result<SheetId, ErrorKind> {
    match spec {
        None => Ok(default),
        Some(s) if s.end.is_some() => Err(ErrorKind::Ref),
        Some(s) => ctx
            .wb
            .resolve_sheet_name(&s.start)
            .map_err(|_| ErrorKind::Ref),
    }
}

fn eval_cell_ref(ctx: &EvalCtx<'_>, spec: Option<&SheetSpec>, cell: CellRef) -> RuntimeValue {
    let sheet = match resolve_sheet(ctx, spec, cell.sheet.unwrap_or(ctx.cell.sheet)) {
        Ok(s) => s,
        Err(e) => return RuntimeValue::error(e),
    };
    RuntimeValue::Ref(Reference::cell(sheet, cell.row, cell.col))
}

fn eval_range_ref(ctx: &EvalCtx<'_>, spec: Option<&SheetSpec>, range: RangeRef) -> RuntimeValue {
    let sheet = match resolve_sheet(ctx, spec, range.start.sheet.unwrap_or(ctx.cell.sheet)) {
        Ok(s) => s,
        Err(e) => return RuntimeValue::error(e),
    };
    RuntimeValue::Ref(Reference::Range {
        sheet,
        start_row: range.start.row,
        start_col: range.start.col,
        end_row: range.end.row,
        end_col: range.end.col,
    })
}

fn eval_threed(ctx: &mut EvalCtx<'_>, sheets: &SheetSpec, inner: &Expr) -> RuntimeValue {
    let span = match sheet_span(ctx.wb, sheets) {
        Ok(s) => s,
        Err(e) => return RuntimeValue::error(e),
    };
    let body = eval_expr(ctx, inner);
    match body {
        RuntimeValue::Ref(Reference::Range {
            start_row,
            start_col,
            end_row,
            end_col,
            ..
        })
        | RuntimeValue::Ref(Reference::ThreeD {
            start_row,
            start_col,
            end_row,
            end_col,
            ..
        }) => RuntimeValue::Ref(Reference::ThreeD {
            sheets: span,
            start_row,
            start_col,
            end_row,
            end_col,
        }),
        RuntimeValue::Ref(Reference::Union(_)) => RuntimeValue::error(ErrorKind::Value),
        RuntimeValue::Scalar(Scalar::Error(e)) => RuntimeValue::error(e),
        _ => RuntimeValue::error(ErrorKind::Value),
    }
}

fn sheet_span(wb: &Workbook, spec: &SheetSpec) -> Result<Vec<SheetId>, ErrorKind> {
    let start = wb
        .resolve_sheet_name(&spec.start)
        .map_err(|_| ErrorKind::Ref)?;
    let end = match &spec.end {
        Some(n) => wb.resolve_sheet_name(n).map_err(|_| ErrorKind::Ref)?,
        None => return Ok(vec![start]),
    };
    let ids: Vec<SheetId> = wb.sheets().map(|s| s.id).collect();
    let i = ids.iter().position(|&x| x == start);
    let j = ids.iter().position(|&x| x == end);
    match (i, j) {
        (Some(a), Some(b)) if a <= b => Ok(ids[a..=b].to_vec()),
        (Some(a), Some(b)) => Ok(ids[b..=a].to_vec()),
        _ => Err(ErrorKind::Ref),
    }
}

fn eval_name(ctx: &mut EvalCtx<'_>, spec: Option<&SheetSpec>, name: &str) -> RuntimeValue {
    if spec.is_none()
        && let Some(v) = ctx.scope_lookup(name)
    {
        return v;
    }
    let sheet = match spec {
        Some(s) => match ctx.wb.resolve_sheet_name(&s.start) {
            Ok(id) => id,
            Err(_) => return RuntimeValue::error(ErrorKind::Name),
        },
        None => ctx.cell.sheet,
    };
    if spec.is_some()
        && let Some(n) = ctx.wb.names().get(NameScope::Sheet(sheet), name)
    {
        return eval_referent(ctx, n.referent.clone());
    }
    if let Some(n) = ctx.wb.names().resolve(sheet, name) {
        return eval_referent(ctx, n.referent.clone());
    }
    RuntimeValue::error(ErrorKind::Name)
}

fn eval_referent(ctx: &mut EvalCtx<'_>, referent: NameReferent) -> RuntimeValue {
    match referent {
        NameReferent::Constant(v) => RuntimeValue::from_stored(v, ctx.wb.intern()),
        NameReferent::Range(r) => {
            let sheet = r.start.sheet.unwrap_or(ctx.cell.sheet);
            if let Some(end) = r.sheet_end {
                let ids: Vec<SheetId> = ctx.wb.sheets().map(|s| s.id).collect();
                let i = ids.iter().position(|&x| x == sheet);
                let j = ids.iter().position(|&x| x == end);
                let span = match (i, j) {
                    (Some(a), Some(b)) if a <= b => ids[a..=b].to_vec(),
                    (Some(a), Some(b)) => ids[b..=a].to_vec(),
                    _ => return RuntimeValue::error(ErrorKind::Ref),
                };
                RuntimeValue::Ref(Reference::ThreeD {
                    sheets: span,
                    start_row: r.start.row,
                    start_col: r.start.col,
                    end_row: r.end.row,
                    end_col: r.end.col,
                })
            } else {
                RuntimeValue::Ref(Reference::Range {
                    sheet,
                    start_row: r.start.row,
                    start_col: r.start.col,
                    end_row: r.end.row,
                    end_col: r.end.col,
                })
            }
        }
        NameReferent::Formula(src) => match parse(&src) {
            Ok(f) => eval_expr(ctx, &f.ast),
            Err(_) => RuntimeValue::error(ErrorKind::Name),
        },
    }
}

fn eval_structured(ctx: &EvalCtx<'_>, sr: &StructuredRef) -> RuntimeValue {
    let table = match &sr.table {
        Some(n) => ctx.wb.tables().get_by_name(n),
        None => table_at(ctx.wb, ctx.cell),
    };
    let Some(table) = table else {
        return RuntimeValue::error(ErrorKind::Name);
    };
    match structured_range(table, sr, ctx.cell) {
        Ok(r) => RuntimeValue::Ref(r),
        Err(e) => RuntimeValue::error(e),
    }
}

fn table_at(wb: &Workbook, cell: CellCoord) -> Option<&Table> {
    wb.tables().iter().find(|t| {
        t.sheet == cell.sheet
            && cell.row >= t.start_row
            && cell.row <= t.end_row
            && cell.col >= t.start_col
            && cell.col <= t.end_col
    })
}

fn col_index(table: &Table, name: &str) -> Option<u16> {
    table
        .columns
        .iter()
        .position(|c| c.name.eq_ignore_ascii_case(name))
        .map(|i| table.start_col + i as u16)
}

fn structured_range(
    table: &Table,
    sr: &StructuredRef,
    formula: CellCoord,
) -> Result<Reference, ErrorKind> {
    let mut item = sr.item;
    let mut this_row = sr.this_row;
    let (c1, c2) = match &sr.columns {
        None => (table.start_col, table.end_col),
        Some(TableColumns::One(n)) => {
            let n = n.trim_matches(|c| c == '[' || c == ']');
            if let Some(it) = TableItem::parse(n) {
                item = Some(it);
                if it == TableItem::ThisRow {
                    this_row = true;
                }
                (table.start_col, table.end_col)
            } else {
                let c = col_index(table, n).ok_or(ErrorKind::Value)?;
                (c, c)
            }
        }
        Some(TableColumns::Span { start, end }) => {
            let start_n = start.trim_matches(|c| c == '[' || c == ']');
            let end_n = end.trim_matches(|c| c == '[' || c == ']');
            if let Some(it) = TableItem::parse(start_n) {
                item = Some(it);
                if it == TableItem::ThisRow {
                    this_row = true;
                }
                let c = col_index(table, end_n).ok_or(ErrorKind::Value)?;
                (c, c)
            } else {
                let a = col_index(table, start_n).ok_or(ErrorKind::Value)?;
                let b = col_index(table, end_n).ok_or(ErrorKind::Value)?;
                (a.min(b), a.max(b))
            }
        }
    };
    let data_start = if table.has_header {
        table.start_row.saturating_add(1)
    } else {
        table.start_row
    };
    let data_end = if table.has_totals {
        table.end_row.saturating_sub(1)
    } else {
        table.end_row
    };
    let (r1, r2) = if this_row || item == Some(TableItem::ThisRow) {
        if formula.sheet != table.sheet
            || formula.row < table.start_row
            || formula.row > table.end_row
        {
            return Err(ErrorKind::Value);
        }
        (formula.row, formula.row)
    } else {
        match item {
            Some(TableItem::All) => (table.start_row, table.end_row),
            Some(TableItem::Headers) => {
                if !table.has_header {
                    return Err(ErrorKind::Ref);
                }
                (table.start_row, table.start_row)
            }
            Some(TableItem::Totals) => {
                if !table.has_totals {
                    return Err(ErrorKind::Ref);
                }
                (table.end_row, table.end_row)
            }
            Some(TableItem::Data) | None => {
                if data_start > data_end {
                    return Err(ErrorKind::Ref);
                }
                (data_start, data_end)
            }
            Some(TableItem::ThisRow) => (formula.row, formula.row),
        }
    };
    Ok(Reference::Range {
        sheet: table.sheet,
        start_row: r1,
        start_col: c1,
        end_row: r2,
        end_col: c2,
    })
}

fn eval_prefix(ctx: &mut EvalCtx<'_>, op: PrefixOp, expr: &Expr) -> RuntimeValue {
    match op {
        PrefixOp::Minus => {
            let raw = eval_expr(ctx, expr);
            let v = materialize_value(ctx, raw);
            ops::unary_minus(v)
        }
        PrefixOp::Plus => {
            let raw = eval_expr(ctx, expr);
            let v = materialize_value(ctx, raw);
            ops::unary_plus(v)
        }
        PrefixOp::ImplicitIntersect => {
            let v = eval_expr(ctx, expr);
            implicit_intersect(ctx, v)
        }
    }
}

fn eval_postfix(ctx: &mut EvalCtx<'_>, expr: &Expr, op: PostfixOp) -> RuntimeValue {
    match op {
        PostfixOp::Percent => {
            let raw = eval_expr(ctx, expr);
            let v = materialize_value(ctx, raw);
            ops::percent(v)
        }
        PostfixOp::Spill => {
            let v = eval_expr(ctx, expr);
            spill_of(ctx, v)
        }
    }
}

fn spill_of(ctx: &EvalCtx<'_>, v: RuntimeValue) -> RuntimeValue {
    match v {
        RuntimeValue::Ref(Reference::Range {
            sheet,
            start_row,
            start_col,
            end_row,
            end_col,
        }) if start_row == end_row && start_col == end_col => {
            if let Some(region) = ctx.spill.region_at(sheet, start_row, start_col) {
                let end_col = region
                    .origin
                    .col
                    .saturating_add((region.cols.saturating_sub(1)) as u16);
                RuntimeValue::Ref(Reference::Range {
                    sheet,
                    start_row: region.origin.row,
                    start_col: region.origin.col,
                    end_row: region
                        .origin
                        .row
                        .saturating_add(region.rows.saturating_sub(1)),
                    end_col,
                })
            } else {
                RuntimeValue::Ref(Reference::cell(sheet, start_row, start_col))
            }
        }
        RuntimeValue::Ref(r) => RuntimeValue::Ref(r),
        other => other,
    }
}

fn eval_binary(ctx: &mut EvalCtx<'_>, op: BinOp, left: &Expr, right: &Expr) -> RuntimeValue {
    match op {
        BinOp::Range => range_op(ctx, left, right),
        BinOp::Isect => isect_op(ctx, left, right),
        BinOp::Union => union_op(ctx, left, right),
        other => {
            let left_v = eval_expr(ctx, left);
            let l = materialize_value(ctx, left_v);
            if let Some(e) = l.error_kind() {
                return RuntimeValue::error(e);
            }
            let right_v = eval_expr(ctx, right);
            let r = materialize_value(ctx, right_v);
            ops::binary(other, l, r)
        }
    }
}

fn as_ref(v: RuntimeValue) -> Option<Reference> {
    match v {
        RuntimeValue::Ref(r) => Some(r),
        _ => None,
    }
}

fn range_op(ctx: &mut EvalCtx<'_>, left: &Expr, right: &Expr) -> RuntimeValue {
    let l = eval_expr(ctx, left);
    let r = eval_expr(ctx, right);
    match (as_ref(l), as_ref(r)) {
        (
            Some(Reference::Range {
                sheet: s1,
                start_row: r1,
                start_col: c1,
                end_row: r1e,
                end_col: c1e,
            }),
            Some(Reference::Range {
                sheet: s2,
                start_row: r2,
                start_col: c2,
                end_row: r2e,
                end_col: c2e,
            }),
        ) if s1 == s2 => {
            let rows = [r1, r1e, r2, r2e];
            let cols = [c1, c1e, c2, c2e];
            RuntimeValue::Ref(Reference::Range {
                sheet: s1,
                start_row: *rows.iter().min().unwrap_or(&r1),
                start_col: *cols.iter().min().unwrap_or(&c1),
                end_row: *rows.iter().max().unwrap_or(&r1e),
                end_col: *cols.iter().max().unwrap_or(&c1e),
            })
        }
        _ => RuntimeValue::error(ErrorKind::Value),
    }
}

fn isect_op(ctx: &mut EvalCtx<'_>, left: &Expr, right: &Expr) -> RuntimeValue {
    let l = eval_expr(ctx, left);
    let r = eval_expr(ctx, right);
    match (as_ref(l), as_ref(r)) {
        (
            Some(Reference::Range {
                sheet: s1,
                start_row: a1,
                start_col: b1,
                end_row: a2,
                end_col: b2,
            }),
            Some(Reference::Range {
                sheet: s2,
                start_row: c1,
                start_col: d1,
                end_row: c2,
                end_col: d2,
            }),
        ) if s1 == s2 => {
            let r1 = a1.max(c1);
            let r2 = a2.min(c2);
            let c1n = b1.max(d1);
            let c2n = b2.min(d2);
            if r1 > r2 || c1n > c2n {
                RuntimeValue::error(ErrorKind::Null)
            } else {
                RuntimeValue::Ref(Reference::Range {
                    sheet: s1,
                    start_row: r1,
                    start_col: c1n,
                    end_row: r2,
                    end_col: c2n,
                })
            }
        }
        _ => RuntimeValue::error(ErrorKind::Null),
    }
}

fn union_op(ctx: &mut EvalCtx<'_>, left: &Expr, right: &Expr) -> RuntimeValue {
    let l = eval_expr(ctx, left);
    let r = eval_expr(ctx, right);
    let mut parts = Vec::new();
    match as_ref(l) {
        Some(Reference::Union(mut u)) => parts.append(&mut u),
        Some(x) => parts.push(x),
        None => return RuntimeValue::error(ErrorKind::Value),
    }
    match as_ref(r) {
        Some(Reference::Union(mut u)) => parts.append(&mut u),
        Some(x) => parts.push(x),
        None => return RuntimeValue::error(ErrorKind::Value),
    }
    RuntimeValue::Ref(Reference::Union(parts))
}

fn materialize_value(ctx: &mut EvalCtx<'_>, v: RuntimeValue) -> RuntimeValue {
    match v {
        RuntimeValue::Ref(r) if r.is_single_cell() => ctx.materialize(RuntimeValue::Ref(r)),
        RuntimeValue::Ref(r) => {
            // Multi-cell: in operator context, materialize the whole array (DA).
            ctx.materialize(RuntimeValue::Ref(r))
        }
        other => other,
    }
}

fn implicit_intersect(ctx: &EvalCtx<'_>, v: RuntimeValue) -> RuntimeValue {
    match v {
        RuntimeValue::Ref(Reference::Range {
            sheet,
            start_row,
            start_col,
            end_row,
            end_col,
        }) => {
            let r1 = start_row.min(end_row);
            let r2 = start_row.max(end_row);
            let c1 = start_col.min(end_col);
            let c2 = start_col.max(end_col);
            let fr = ctx.cell.row;
            let fc = ctx.cell.col;
            if r1 == r2 && c1 == c2 {
                return RuntimeValue::Scalar(ctx.read_cell(sheet, r1, c1));
            }
            if r1 == r2 {
                // One row: take the formula's column if it lands in the range,
                // else the left cell (Excel implicit intersection of a row).
                let col = if fc >= c1 && fc <= c2 { fc } else { c1 };
                return RuntimeValue::Scalar(ctx.read_cell(sheet, r1, col));
            }
            if c1 == c2 {
                let row = if fr >= r1 && fr <= r2 { fr } else { r1 };
                return RuntimeValue::Scalar(ctx.read_cell(sheet, row, c1));
            }
            RuntimeValue::error(ErrorKind::Value)
        }
        RuntimeValue::Array(a) => {
            RuntimeValue::Scalar(a.values.first().cloned().unwrap_or(Scalar::Empty))
        }
        other => other,
    }
}

fn eval_call(ctx: &mut EvalCtx<'_>, callee: &Callee, args: &[Option<Expr>]) -> RuntimeValue {
    match callee {
        Callee::Name(name) => eval_named_call(ctx, name, args),
        Callee::Expr(e) => {
            let v = eval_expr(ctx, e);
            let argv = eval_args(ctx, args);
            lambda::apply_value(ctx, v, &argv)
        }
    }
}

fn eval_named_call(ctx: &mut EvalCtx<'_>, name: &str, args: &[Option<Expr>]) -> RuntimeValue {
    let upper = name.to_ascii_uppercase();
    match upper.as_str() {
        "LET" => return lambda::eval_let(ctx, args),
        "LAMBDA" => return lambda::eval_lambda_def(ctx, args),
        "ISOMITTED" => return lambda::eval_isomitted(ctx, args),
        _ => {}
    }
    if let Some(v) = ctx.scope_lookup(name) {
        let argv = eval_args(ctx, args);
        return lambda::apply_value(ctx, v, &argv);
    }
    if let Some(def) = ctx.registry.lookup(name) {
        if args.len() < def.min_args as usize || args.len() > def.max_args as usize {
            return RuntimeValue::error(ErrorKind::Value);
        }
        return dispatch_fn(ctx, def, args);
    }
    // Defined name that is a lambda / formula.
    if let Some(n) = ctx.wb.names().resolve(ctx.cell.sheet, name) {
        let referent = n.referent.clone();
        let callee_v = eval_referent(ctx, referent);
        let argv = eval_args(ctx, args);
        return lambda::apply_value(ctx, callee_v, &argv);
    }
    registry::name_error()
}

fn dispatch_fn(ctx: &mut EvalCtx<'_>, def: &FnDef, args: &[Option<Expr>]) -> RuntimeValue {
    match def.body {
        FnBody::Lazy(eval) => eval(ctx, args),
        FnBody::Eager(eval) => {
            let argv = eval_args(ctx, args);
            if def.async_node {
                eval_async(ctx, def, &argv)
            } else if def.array_lift == ArrayLift::All {
                eval_array_lifted(ctx, eval, &argv)
            } else {
                eval(ctx, &argv)
            }
        }
    }
}

fn eval_array_lifted(
    ctx: &mut EvalCtx<'_>,
    eval: fn(&mut EvalCtx<'_>, &[ArgVal]) -> RuntimeValue,
    args: &[ArgVal],
) -> RuntimeValue {
    let args: Vec<ArgVal> = args
        .iter()
        .map(|arg| ArgVal {
            omitted: arg.omitted,
            value: ctx.materialize(arg.value.clone()),
        })
        .collect();
    let mut rows = 1u32;
    let mut cols = 1u32;
    for arg in &args {
        if let RuntimeValue::Array(array) = &arg.value {
            rows = rows.max(array.rows);
            cols = cols.max(array.cols);
        }
    }
    if rows == 1 && cols == 1 {
        return eval(ctx, &args);
    }

    let mut values = Vec::with_capacity((rows as usize).saturating_mul(cols as usize));
    for row in 0..rows {
        for col in 0..cols {
            let cell_args: Vec<ArgVal> = args
                .iter()
                .map(|arg| ArgVal {
                    omitted: arg.omitted,
                    value: RuntimeValue::Scalar(lifted_scalar(&arg.value, row, col)),
                })
                .collect();
            let result = eval(ctx, &cell_args);
            values.push(match result {
                RuntimeValue::Scalar(scalar) => scalar,
                RuntimeValue::Array(array) if array.rows == 1 && array.cols == 1 => {
                    array.values.first().cloned().unwrap_or(Scalar::Empty)
                }
                RuntimeValue::Array(_) | RuntimeValue::Lambda(_) | RuntimeValue::Ref(_) => {
                    Scalar::Error(ErrorKind::Value)
                }
            });
        }
    }
    RuntimeValue::array(rows, cols, values)
}

fn lifted_scalar(value: &RuntimeValue, row: u32, col: u32) -> Scalar {
    match value {
        RuntimeValue::Scalar(scalar) => scalar.clone(),
        RuntimeValue::Array(array) => {
            let row = if array.rows == 1 {
                0
            } else if row < array.rows {
                row
            } else {
                return Scalar::Error(ErrorKind::Na);
            };
            let col = if array.cols == 1 {
                0
            } else if col < array.cols {
                col
            } else {
                return Scalar::Error(ErrorKind::Na);
            };
            let index = (row as usize)
                .saturating_mul(array.cols as usize)
                .saturating_add(col as usize);
            array.values.get(index).cloned().unwrap_or(Scalar::Empty)
        }
        RuntimeValue::Lambda(_) | RuntimeValue::Ref(_) => Scalar::Error(ErrorKind::Value),
    }
}

fn eval_args(ctx: &mut EvalCtx<'_>, args: &[Option<Expr>]) -> Vec<ArgVal> {
    args.iter()
        .map(|a| match a {
            None => ArgVal {
                omitted: true,
                value: RuntimeValue::Scalar(Scalar::Empty),
            },
            Some(e) => ArgVal {
                omitted: false,
                value: eval_expr(ctx, e),
            },
        })
        .collect()
}

fn eval_async(ctx: &mut EvalCtx<'_>, def: &FnDef, args: &[ArgVal]) -> RuntimeValue {
    let Some(provider) = ctx.async_provider else {
        ctx.mark_pending();
        return RuntimeValue::error(ErrorKind::GettingData);
    };
    let materialized: Vec<ArgVal> = args
        .iter()
        .map(|arg| ArgVal {
            omitted: arg.omitted,
            value: ctx.materialize(arg.value.clone()),
        })
        .collect();
    eval_async_fn(ctx, def, &materialized, provider)
}

/// Evaluate a formula AST for `cell` and return the runtime result.
pub fn eval_formula(
    wb: &Workbook,
    registry: &FnRegistry,
    spill: &SpillTable,
    cell: CellCoord,
    ast: &Expr,
    pass: u32,
) -> (RuntimeValue, EvalFlags) {
    eval_formula_in(wb, registry, spill, cell, ast, pass, PassEnv::default())
}

/// [`eval_formula`] with an explicit pass environment.
pub fn eval_formula_in(
    wb: &Workbook,
    registry: &FnRegistry,
    spill: &SpillTable,
    cell: CellCoord,
    ast: &Expr,
    pass: u32,
    env: PassEnv,
) -> (RuntimeValue, EvalFlags) {
    let mut ctx = EvalCtx::new(wb, registry, spill, cell, pass).with_pass_env(env);
    let raw = eval_expr(&mut ctx, ast);
    let value = prepare_result(&ctx, raw);
    let (pending, stale, hint, dynamic) = ctx.take_flags();
    (
        value,
        EvalFlags {
            pending_async: pending,
            stale,
            hint,
            dynamic,
        },
    )
}

/// Flags collected while evaluating one cell.
#[derive(Clone, Debug, Default)]
pub struct EvalFlags {
    /// Async node returned pending.
    pub pending_async: bool,
    /// Cell (or dependents) should show stale.
    pub stale: bool,
    /// Provider failure hint.
    pub hint: Option<String>,
    /// Dynamic refs resolved this pass (`INDIRECT`/`OFFSET`).
    pub dynamic: Vec<Reference>,
}

fn prepare_result(ctx: &EvalCtx<'_>, raw: RuntimeValue) -> RuntimeValue {
    match raw {
        RuntimeValue::Ref(r) => {
            // Formula result: single cell → scalar; multi-cell → array (spill).
            ctx.materialize(RuntimeValue::Ref(r))
        }
        RuntimeValue::Lambda(l) => RuntimeValue::Lambda(l),
        other => other,
    }
}

/// Format a runtime value for corpus comparison.
#[must_use]
pub fn format_runtime(v: &RuntimeValue) -> String {
    match v {
        RuntimeValue::Scalar(s) => format_scalar(s),
        RuntimeValue::Array(a) => format_array(a),
        RuntimeValue::Lambda(_) => ErrorKind::Calc.as_str().to_string(),
        RuntimeValue::Ref(_) => "#REF!".into(),
    }
}

fn format_scalar(s: &Scalar) -> String {
    match s {
        Scalar::Empty => String::new(),
        Scalar::Number(n) => format_number(*n),
        Scalar::Bool(true) => "TRUE".into(),
        Scalar::Bool(false) => "FALSE".into(),
        Scalar::Text(t) => t.to_string(),
        Scalar::Error(e) => e.as_str().to_string(),
    }
}

fn format_number(n: f64) -> String {
    if !n.is_finite() {
        return ErrorKind::Num.as_str().to_string();
    }
    if n == 0.0 {
        return "0".into();
    }
    if n.fract() == 0.0 && n.abs() < 1e15 {
        return format!("{n:.0}");
    }
    crate::numfmt::general(n)
}

fn format_array(a: &RuntimeArray) -> String {
    let mut out = String::from("{");
    let cols = a.cols as usize;
    for r in 0..a.rows as usize {
        if r > 0 {
            out.push(';');
        }
        for c in 0..cols {
            if c > 0 {
                out.push(',');
            }
            let index = r.saturating_mul(cols).saturating_add(c);
            out.push_str(&format_scalar(
                a.values.get(index).unwrap_or(&Scalar::Empty),
            ));
        }
    }
    out.push('}');
    out
}

/// Address helper used by diagnostics.
#[must_use]
pub fn a1_of(row: u32, col: u16) -> String {
    match col_to_letters(col) {
        Ok(l) => format!("{}{}", l, row.saturating_add(1)),
        Err(_) => format!(
            "R{}C{}",
            row.saturating_add(1),
            u32::from(col).saturating_add(1)
        ),
    }
}

/// Shared AST cache keyed by interned source.
#[derive(Clone, Debug, Default)]
pub struct AstCache {
    by_source: FxHashMap<String, Formula>,
}

impl AstCache {
    /// Empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse `source` or return a cached tree.
    pub fn get_or_parse(&mut self, source: &str) -> Result<Formula, ErrorKind> {
        if let Some(f) = self.by_source.get(source) {
            return Ok(f.clone());
        }
        let f = parse(source).map_err(|_| ErrorKind::Name)?;
        self.by_source.insert(source.to_string(), f.clone());
        Ok(f)
    }

    /// Borrow a cached formula without inserting.
    #[must_use]
    pub fn peek(&self, source: &str) -> Option<&Formula> {
        self.by_source.get(source)
    }
}

/// Dispatch an async function through `provider`.
pub fn eval_async_fn(
    ctx: &mut EvalCtx<'_>,
    def: &FnDef,
    args: &[ArgVal],
    provider: &dyn crate::recalc::AsyncNodeProvider,
) -> RuntimeValue {
    let req = AsyncRequest {
        name: def.name.to_string(),
        cell: ctx.cell,
        args: args.to_vec(),
    };
    let key = ContentHash::of_args(def.name, args);
    match provider.evaluate(key, &req) {
        AsyncState::Ready(v) => RuntimeValue::from_stored(v, ctx.wb.intern()),
        AsyncState::Pending { cached } => {
            ctx.mark_pending();
            match cached {
                Some(v) => RuntimeValue::from_stored(v, ctx.wb.intern()),
                None => RuntimeValue::error(ErrorKind::GettingData),
            }
        }
        AsyncState::Failed { hint } => {
            ctx.fail_hint(hint);
            RuntimeValue::error(ErrorKind::Na)
        }
    }
}
