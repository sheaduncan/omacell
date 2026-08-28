//! Incremental, deterministic, parallel recalculation (F-3.6, F-3.7, §11.5).

use std::sync::Arc;
use std::time::Instant;

use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::coerce::Scalar;
use crate::error::ErrorKind;
use crate::eval::{
    ArgVal, AstCache, EvalCtx, EvalFlags, FnDef, FnRegistry, RuntimeValue, eval_async_fn,
    eval_expr, eval_formula, format_runtime,
};
use crate::graph::{CellCoord, DepGraph};
use crate::intern::ArrayPayload;
use crate::spill::{SpillRegion, SpillTable, blocks_spill};
use crate::storage::{CellFlags, CellSlot};
use crate::style::StyleId;
use crate::value::{Array2D, Value};
use crate::workbook::{CalcMode, Workbook};

/// Recalculation modes (F-3.6). Alias of workbook settings.
pub type RecalcMode = CalcMode;

/// Content-addressed key for async nodes (A-3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ContentHash(pub u64);

impl ContentHash {
    /// Hash a function name, cell, and pass.
    #[must_use]
    pub fn of(name: &str, cell: CellCoord, _pass: u32) -> Self {
        // Pass is ignored: AI nodes are non-volatile (A-3.2); the cache key is
        // (function, cell). Argument identity is WP-23.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in name.as_bytes() {
            h = h.wrapping_mul(0x1000_0000_01b3) ^ u64::from(*b);
        }
        h ^= u64::from(cell.sheet.index()).wrapping_shl(32);
        h ^= u64::from(cell.row);
        h ^= u64::from(cell.col).wrapping_shl(16);
        Self(h)
    }
}

/// Request passed to [`AsyncNodeProvider::evaluate`].
#[derive(Clone, Debug)]
pub struct AsyncRequest {
    /// Function name (`AI`, …).
    pub name: String,
    /// Formula cell.
    pub cell: CellCoord,
}

/// Result of an async node evaluation (sync trait; core has no tokio).
#[derive(Clone, Debug)]
pub enum AsyncState {
    /// Still running; show `cached` (or `#GETTING_DATA`) as stale.
    Pending {
        /// Previously cached value, if any.
        cached: Option<Value>,
    },
    /// Finished.
    Ready(Value),
    /// Failed; cell becomes `#N/A` and `hint` is surfaced on [`RecalcResult`].
    Failed {
        /// Machine-readable hint (A-3.6).
        hint: String,
    },
}

/// Provider for async graph nodes (AI cells). Real providers are WP-23.
pub trait AsyncNodeProvider: Send + Sync {
    /// Evaluate (or start) the request.
    fn evaluate(&self, key: ContentHash, req: &AsyncRequest) -> AsyncState;
}

/// In-memory mock: first call pending, later calls ready.
#[derive(Debug, Default)]
pub struct MockAsyncProvider {
    inner: std::sync::Mutex<FxHashMap<ContentHash, u32>>,
    /// Value returned when ready.
    pub ready: Value,
}

impl MockAsyncProvider {
    /// New mock that yields `ready` on the second evaluate.
    #[must_use]
    pub fn new(ready: Value) -> Self {
        Self {
            inner: std::sync::Mutex::new(FxHashMap::default()),
            ready,
        }
    }

    fn bump(&self, key: ContentHash) -> u32 {
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let e = g.entry(key).or_insert(0);
        *e += 1;
        *e
    }
}

impl AsyncNodeProvider for MockAsyncProvider {
    fn evaluate(&self, key: ContentHash, _req: &AsyncRequest) -> AsyncState {
        if self.bump(key) <= 1 {
            AsyncState::Pending { cached: None }
        } else {
            AsyncState::Ready(self.ready)
        }
    }
}

/// Outcome of a recalc pass.
#[derive(Clone, Debug, Default)]
pub struct RecalcResult {
    /// Formula cells evaluated.
    pub cells_evaluated: u64,
    /// Wall time.
    pub elapsed_ms: u64,
    /// Circular cells (sorted). Empty when iteration resolved them.
    pub circular: Vec<CellCoord>,
    /// Spill origins that were blocked: (origin, blocker).
    pub spill_blocked: Vec<(CellCoord, CellCoord)>,
    /// Async cells still pending.
    pub pending_async: Vec<CellCoord>,
    /// Cells marked stale this pass.
    pub stale: Vec<CellCoord>,
    /// Provider hints (cell, hint).
    pub async_hints: Vec<(CellCoord, String)>,
}

/// Recalculation engine: graph, AST cache, registry, spill table, thread pool.
///
/// ```
/// use omacell_core::eval::FnRegistry;
/// use omacell_core::recalc::RecalcEngine;
/// use omacell_core::workbook::Workbook;
/// let mut wb = Workbook::new();
/// let sheet = wb.active_sheet();
/// wb.set_formula_text(sheet, 0, 0, "=1+1").unwrap();
/// let mut engine = RecalcEngine::new(FnRegistry::new());
/// let result = engine.recalc_full(&mut wb);
/// assert!(result.cells_evaluated >= 1);
/// ```
pub struct RecalcEngine {
    graph: DepGraph,
    asts: AstCache,
    registry: FnRegistry,
    spill: SpillTable,
    dirty: FxHashSet<CellCoord>,
    threads: usize,
    pool: Option<rayon::ThreadPool>,
    pass: u32,
    async_provider: Option<Arc<dyn AsyncNodeProvider>>,
    /// Last dynamic-resolved refs per cell.
    dynamic_edges: FxHashMap<CellCoord, Vec<CellCoord>>,
}

impl RecalcEngine {
    /// Engine with the given registry (empty = every unknown fn is `#NAME?`).
    #[must_use]
    pub fn new(registry: FnRegistry) -> Self {
        Self {
            graph: DepGraph::new(),
            asts: AstCache::new(),
            registry,
            spill: SpillTable::new(),
            dirty: FxHashSet::default(),
            threads: 1,
            pool: None,
            pass: 0,
            async_provider: None,
            dynamic_edges: FxHashMap::default(),
        }
    }

    /// Borrow the registry (WP-05 registers here).
    pub fn registry_mut(&mut self) -> &mut FnRegistry {
        &mut self.registry
    }

    /// Borrow the registry.
    #[must_use]
    pub fn registry(&self) -> &FnRegistry {
        &self.registry
    }

    /// Dependency graph.
    #[must_use]
    pub fn graph(&self) -> &DepGraph {
        &self.graph
    }

    /// Spill table (for `A1#`).
    #[must_use]
    pub fn spill(&self) -> &SpillTable {
        &self.spill
    }

    /// Install an async provider.
    pub fn set_async_provider(&mut self, provider: Arc<dyn AsyncNodeProvider>) {
        self.async_provider = Some(provider);
    }

    /// Pin the rayon pool to `n` threads (determinism tests use 1 vs 8).
    pub fn set_threads(&mut self, n: usize) {
        let n = n.max(1);
        self.threads = n;
        self.pool = rayon::ThreadPoolBuilder::new().num_threads(n).build().ok();
    }

    /// Current thread count.
    #[must_use]
    pub fn threads(&self) -> usize {
        self.threads
    }

    /// Rebuild the graph from `wb` and dirty every formula cell.
    pub fn rebuild(&mut self, wb: &Workbook) {
        self.graph.rebuild(wb, &mut self.asts);
        self.dirty.clear();
        self.dirty.extend(self.graph.formula_cells());
        self.spill = SpillTable::new();
        self.dynamic_edges.clear();
    }

    /// Record that `coord` changed (value or formula).
    pub fn notify_edit(&mut self, wb: &Workbook, coord: CellCoord) {
        let dependents = self.graph.dependents(coord);
        let was_formula = self.graph.calc_chain().contains(&coord);
        if let Ok(Some(slot)) = wb.get(coord.sheet, coord.row, coord.col) {
            if let Some(fid) = slot.formula {
                self.graph.upsert_formula(wb, &mut self.asts, coord, fid);
            } else if was_formula {
                self.graph.remove_node(coord);
            }
        } else if was_formula {
            self.graph.remove_node(coord);
        }
        self.dirty.insert(coord);
        self.dirty.extend(dependents);
        if let Some(extra) = self.dynamic_edges.get(&coord).cloned() {
            self.dirty.extend(extra);
        }
        // Dynamic cells that resolved to this cell last pass.
        let dyn_watchers: Vec<CellCoord> = self
            .dynamic_edges
            .iter()
            .filter_map(|(c, refs)| {
                if refs.contains(&coord) {
                    Some(*c)
                } else {
                    None
                }
            })
            .collect();
        self.dirty.extend(dyn_watchers);
    }

    /// Full recalc of every formula cell.
    pub fn recalc_full(&mut self, wb: &mut Workbook) -> RecalcResult {
        self.rebuild(wb);
        self.run(wb, true)
    }

    /// Rebuild graph then full recalc.
    pub fn recalc_rebuild(&mut self, wb: &mut Workbook) -> RecalcResult {
        self.recalc_full(wb)
    }

    /// Incremental recalc of the dirty set (no-op in manual mode).
    pub fn recalc_incremental(&mut self, wb: &mut Workbook) -> RecalcResult {
        match wb.settings().calc_mode {
            CalcMode::Manual => RecalcResult::default(),
            CalcMode::Automatic | CalcMode::AutomaticExceptTables => {
                if self.graph.is_empty() {
                    self.rebuild(wb);
                }
                for v in self.graph.volatiles() {
                    self.dirty.insert(v);
                }
                self.run(wb, false)
            }
        }
    }

    fn run(&mut self, wb: &mut Workbook, full: bool) -> RecalcResult {
        let t0 = Instant::now();
        self.pass = self.pass.saturating_add(1);
        let mut dirty: Vec<CellCoord> = if full {
            self.graph.formula_cells()
        } else {
            let seeds: Vec<CellCoord> = self.dirty.iter().copied().collect();
            self.graph.propagate(seeds)
        };
        self.dirty.clear();
        dirty.sort_unstable();
        dirty.dedup();
        for c in &dirty {
            if let Ok(Some(slot)) = wb.get(c.sheet, c.row, c.col)
                && let Some(fid) = slot.formula
                && let Some(src) = wb.intern().formulas.get(fid)
            {
                let _ = self.asts.get_or_parse(src);
            }
        }

        let mut circular = self.graph.circular_set(&dirty);
        let iteration = wb.settings().iteration;
        let mut evaluated = 0u64;
        let mut spill_blocked = Vec::new();
        let mut pending_async = Vec::new();
        let mut stale = Vec::new();
        let mut async_hints = Vec::new();

        if !circular.is_empty() && !iteration.enabled {
            for c in &circular {
                commit_scalar(wb, *c, Scalar::Number(0.0), CellFlags::DEFAULT);
            }
            evaluated += circular.len() as u64;
        }

        let gens = self.graph.generations(&dirty);
        for generation in gens {
            evaluated += self.eval_generation(
                wb,
                &generation,
                &mut spill_blocked,
                &mut pending_async,
                &mut stale,
                &mut async_hints,
            ) as u64;
        }

        if iteration.enabled && !circular.is_empty() {
            evaluated += self.iterate_cycle(
                wb,
                &circular,
                iteration.max_iterations,
                iteration.max_change,
            ) as u64;
            circular.clear();
        }

        // Dynamic nodes: record resolved refs for the next dirty pass.
        // (Evaluated as part of generations when they have no static preds.)

        RecalcResult {
            cells_evaluated: evaluated,
            elapsed_ms: t0.elapsed().as_millis() as u64,
            circular,
            spill_blocked,
            pending_async,
            stale,
            async_hints,
        }
    }

    fn eval_generation(
        &mut self,
        wb: &mut Workbook,
        generation: &[CellCoord],
        spill_blocked: &mut Vec<(CellCoord, CellCoord)>,
        pending_async: &mut Vec<CellCoord>,
        stale: &mut Vec<CellCoord>,
        async_hints: &mut Vec<(CellCoord, String)>,
    ) -> usize {
        if generation.is_empty() {
            return 0;
        }
        let mut results: Vec<(CellCoord, RuntimeValue, EvalFlags, bool)> = {
            let wb_ref: &Workbook = wb;
            let registry = &self.registry;
            let spill = &self.spill;
            let asts = &self.asts;
            let pass = self.pass;
            let provider = self.async_provider.clone();
            let eval_one = |cell: CellCoord| {
                eval_one_cell(
                    wb_ref,
                    registry,
                    spill,
                    asts,
                    pass,
                    provider.as_deref(),
                    cell,
                )
            };
            if let Some(pool) = &self.pool {
                pool.install(|| generation.par_iter().filter_map(|c| eval_one(*c)).collect())
            } else if self.threads <= 1 {
                generation.iter().filter_map(|c| eval_one(*c)).collect()
            } else {
                generation.par_iter().filter_map(|c| eval_one(*c)).collect()
            }
        };
        results.sort_by_key(|(c, _, _, _)| *c);

        let undo = wb.undo_log_mut().is_enabled();
        wb.undo_log_mut().set_enabled(false);
        for (cell, value, flags, cse) in results {
            if flags.pending_async {
                pending_async.push(cell);
            }
            if flags.stale {
                stale.push(cell);
            }
            if let Some(h) = flags.hint {
                async_hints.push((cell, h));
            }
            if !flags.dynamic.is_empty() {
                let mut coords = Vec::new();
                for r in &flags.dynamic {
                    collect_ref_cells(r, &mut coords);
                }
                coords.sort_unstable();
                coords.dedup();
                self.dynamic_edges.insert(cell, coords);
            }
            if let Some(block) = commit_value(wb, &mut self.spill, cell, value, cse, flags.stale) {
                spill_blocked.push((cell, block));
            }
        }
        wb.undo_log_mut().set_enabled(undo);
        generation.len()
    }

    fn iterate_cycle(
        &mut self,
        wb: &mut Workbook,
        cycle: &[CellCoord],
        max_iter: u32,
        max_change: f64,
    ) -> usize {
        let mut n = 0usize;
        for _ in 0..max_iter.max(1) {
            let mut max_delta = 0.0f64;
            for &cell in cycle {
                let before = match wb.get(cell.sheet, cell.row, cell.col) {
                    Ok(Some(s)) => match s.value {
                        Value::Number(v) => v,
                        _ => 0.0,
                    },
                    _ => 0.0,
                };
                let Some(fid) = wb
                    .get(cell.sheet, cell.row, cell.col)
                    .ok()
                    .flatten()
                    .and_then(|s| s.formula)
                else {
                    continue;
                };
                let Some(src) = wb.intern().formulas.get(fid).map(str::to_string) else {
                    continue;
                };
                let Ok(formula) = self.asts.get_or_parse(&src) else {
                    continue;
                };
                let (value, _) = eval_formula(
                    wb,
                    &self.registry,
                    &self.spill,
                    cell,
                    &formula.ast,
                    self.pass,
                );
                let after = match &value {
                    RuntimeValue::Scalar(Scalar::Number(v)) => *v,
                    _ => 0.0,
                };
                commit_runtime(wb, cell, &value, false, false);
                max_delta = max_delta.max((after - before).abs());
                n += 1;
            }
            if max_delta <= max_change {
                break;
            }
        }
        n
    }
}

fn eval_one_cell(
    wb: &Workbook,
    registry: &FnRegistry,
    spill: &SpillTable,
    asts: &AstCache,
    pass: u32,
    provider: Option<&dyn AsyncNodeProvider>,
    cell: CellCoord,
) -> Option<(CellCoord, RuntimeValue, EvalFlags, bool)> {
    let slot = wb.get(cell.sheet, cell.row, cell.col).ok()??;
    let fid = slot.formula?;
    let src = wb.intern().formulas.get(fid)?;
    let cse = slot.flags.array();
    let formula = asts.peek(src)?;
    let mut ctx = EvalCtx::new(wb, registry, spill, cell, pass);
    let raw = if let Some(def) = async_def(registry, &formula.ast) {
        let argv: Vec<ArgVal> = Vec::new();
        if let Some(p) = provider {
            eval_async_fn(&mut ctx, def, &argv, p)
        } else {
            ctx.mark_pending();
            RuntimeValue::error(ErrorKind::Na)
        }
    } else {
        eval_expr(&mut ctx, &formula.ast)
    };
    let (pending, is_stale, hint, dynamic) = ctx.take_flags();
    let value = match raw {
        RuntimeValue::Ref(r) => ctx.materialize(RuntimeValue::Ref(r)),
        other => other,
    };
    Some((
        cell,
        value,
        EvalFlags {
            pending_async: pending,
            stale: is_stale,
            hint,
            dynamic,
        },
        cse,
    ))
}

fn async_def<'a>(registry: &'a FnRegistry, ast: &crate::formula::Expr) -> Option<&'a FnDef> {
    match &ast.kind {
        crate::formula::ExprKind::Call {
            callee: crate::formula::Callee::Name(n),
            ..
        } => {
            let d = registry.lookup(n)?;
            if d.async_node { Some(d) } else { None }
        }
        _ => None,
    }
}

fn collect_ref_cells(r: &crate::eval::Reference, out: &mut Vec<CellCoord>) {
    match r {
        crate::eval::Reference::Range {
            sheet,
            start_row,
            start_col,
            end_row,
            end_col,
        } => {
            let r1 = *start_row.min(end_row);
            let r2 = *start_row.max(end_row);
            let c1 = *start_col.min(end_col);
            let c2 = *start_col.max(end_col);
            // Don't explode whole columns; store the four corners as a proxy plus
            // every cell for modest ranges.
            let n = (u64::from(r2 - r1) + 1) * (u64::from(c2 - c1) + 1);
            if n <= 4096 {
                for row in r1..=r2 {
                    for col in c1..=c2 {
                        out.push(CellCoord::new(*sheet, row, col));
                    }
                }
            } else {
                out.push(CellCoord::new(*sheet, r1, c1));
            }
        }
        crate::eval::Reference::Union(parts) => {
            for p in parts {
                collect_ref_cells(p, out);
            }
        }
        crate::eval::Reference::ThreeD { sheets, .. } => {
            for s in sheets {
                out.push(CellCoord::new(*s, 0, 0));
            }
        }
    }
}

fn commit_value(
    wb: &mut Workbook,
    spill: &mut SpillTable,
    cell: CellCoord,
    value: RuntimeValue,
    cse: bool,
    stale: bool,
) -> Option<CellCoord> {
    spill.clear_ghosts(wb, cell);
    spill.remove(cell);
    match value {
        RuntimeValue::Lambda(_) => {
            commit_scalar(wb, cell, Scalar::Error(ErrorKind::Calc), flags(stale));
            None
        }
        RuntimeValue::Scalar(s) => {
            commit_scalar(wb, cell, s, flags(stale));
            None
        }
        RuntimeValue::Array(a) if cse || (a.rows == 1 && a.cols == 1) => {
            let s = a.values.first().cloned().unwrap_or(Scalar::Empty);
            commit_scalar(wb, cell, s, flags(stale).with(CellFlags::ARRAY, cse));
            None
        }
        RuntimeValue::Array(a) => spill_array(wb, spill, cell, &a, stale),
        RuntimeValue::Ref(_) => {
            commit_scalar(wb, cell, Scalar::Error(ErrorKind::Value), flags(stale));
            None
        }
    }
}

fn flags(stale: bool) -> CellFlags {
    CellFlags::DEFAULT.with(CellFlags::STALE, stale)
}

fn spill_array(
    wb: &mut Workbook,
    spill: &mut SpillTable,
    origin: CellCoord,
    a: &crate::eval::RuntimeArray,
    stale: bool,
) -> Option<CellCoord> {
    let mut blocker = None;
    for dr in 0..a.rows {
        for dc in 0..a.cols {
            if dr == 0 && dc == 0 {
                continue;
            }
            let row = origin.row.saturating_add(dr);
            let col = origin.col.saturating_add(dc as u16);
            if row >= crate::limits::MAX_ROWS
                || u32::from(col) >= u32::from(crate::limits::MAX_COLS)
            {
                blocker = Some(CellCoord::new(origin.sheet, row, col));
                break;
            }
            match wb.get(origin.sheet, row, col) {
                Ok(Some(slot)) if blocks_spill(slot) => {
                    blocker = Some(CellCoord::new(origin.sheet, row, col));
                    break;
                }
                Err(_) => {
                    blocker = Some(CellCoord::new(origin.sheet, row, col));
                    break;
                }
                _ => {}
            }
        }
        if blocker.is_some() {
            break;
        }
    }
    if let Some(b) = blocker {
        commit_scalar(wb, origin, Scalar::Error(ErrorKind::Spill), flags(stale));
        spill.insert(SpillRegion {
            origin,
            rows: a.rows,
            cols: a.cols,
            blocked_by: Some(b),
        });
        return Some(b);
    }
    // Origin stores the top-left scalar; ghosts get the rest.
    let first = a.values.first().cloned().unwrap_or(Scalar::Empty);
    commit_scalar(wb, origin, first, flags(stale));
    let cols = a.cols as usize;
    for dr in 0..a.rows {
        for dc in 0..a.cols {
            if dr == 0 && dc == 0 {
                continue;
            }
            let idx = (dr as usize) * cols + (dc as usize);
            let s = a.values.get(idx).cloned().unwrap_or(Scalar::Empty);
            let row = origin.row + dr;
            let col = origin.col + dc as u16;
            write_ghost(wb, origin.sheet, row, col, s);
        }
    }
    spill.insert(SpillRegion {
        origin,
        rows: a.rows,
        cols: a.cols,
        blocked_by: None,
    });
    None
}

fn write_ghost(wb: &mut Workbook, sheet: crate::addr::SheetId, row: u32, col: u16, s: Scalar) {
    let value = intern_scalar(wb, s);
    let slot = CellSlot {
        value,
        formula: None,
        style: StyleId::DEFAULT,
        flags: CellFlags::DEFAULT.with(CellFlags::SPILL, true),
    };
    let _ = wb.set_slot(sheet, row, col, slot);
    release_intern_extra(wb, value);
}

fn commit_runtime(
    wb: &mut Workbook,
    cell: CellCoord,
    value: &RuntimeValue,
    cse: bool,
    stale: bool,
) {
    match value {
        RuntimeValue::Scalar(s) => commit_scalar(
            wb,
            cell,
            s.clone(),
            flags(stale).with(CellFlags::ARRAY, cse),
        ),
        RuntimeValue::Array(a) => {
            let s = a.values.first().cloned().unwrap_or(Scalar::Empty);
            commit_scalar(wb, cell, s, flags(stale).with(CellFlags::ARRAY, cse));
        }
        RuntimeValue::Lambda(_) => {
            commit_scalar(wb, cell, Scalar::Error(ErrorKind::Calc), flags(stale));
        }
        RuntimeValue::Ref(_) => {
            commit_scalar(wb, cell, Scalar::Error(ErrorKind::Value), flags(stale));
        }
    }
}

fn commit_scalar(wb: &mut Workbook, cell: CellCoord, s: Scalar, flags: CellFlags) {
    let value = intern_scalar(wb, s);
    let mut slot = wb
        .get(cell.sheet, cell.row, cell.col)
        .ok()
        .flatten()
        .copied()
        .unwrap_or_else(CellSlot::empty);
    slot.value = value;
    slot.flags = flags
        .with(CellFlags::DIRTY, false)
        .with(CellFlags::ARRAY, slot.flags.array())
        .with(CellFlags::SPILL, false);
    let _ = wb.set_slot(cell.sheet, cell.row, cell.col, slot);
    release_intern_extra(wb, value);
}

fn release_intern_extra(wb: &mut Workbook, value: Value) {
    match value {
        Value::Text(id) => wb.release_text(id),
        Value::Array(id) => wb.release_array(id),
        _ => {}
    }
}

fn intern_scalar(wb: &mut Workbook, s: Scalar) -> Value {
    match s {
        Scalar::Empty => Value::Empty,
        Scalar::Number(n) => {
            if n.is_finite() {
                Value::Number(n)
            } else {
                Value::Error(ErrorKind::Num)
            }
        }
        Scalar::Bool(b) => Value::Bool(b),
        Scalar::Text(t) => Value::Text(wb.intern_text(&t)),
        Scalar::Error(e) => Value::Error(e),
    }
}

/// Display a stored cell for corpora (resolves interned text / arrays).
#[must_use]
pub fn format_cell(wb: &Workbook, sheet: crate::addr::SheetId, row: u32, col: u16) -> String {
    let Ok(Some(slot)) = wb.get(sheet, row, col) else {
        return String::new();
    };
    format_runtime(&RuntimeValue::from_stored(slot.value, wb.intern()))
}

/// Helper used by benches: intern a 1×1 array (kept so ArrayPayload is referenced).
#[allow(dead_code)]
fn _array_payload(values: Vec<Value>) -> Result<ArrayPayload, crate::error::CoreError> {
    let n = values.len() as u32;
    ArrayPayload::new(Array2D::new(1, n.max(1))?, values)
}
