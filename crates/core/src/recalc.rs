//! Incremental, deterministic, parallel recalculation (F-3.6, F-3.7, §11.5).

use std::sync::Arc;
use std::time::Instant;

use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::coerce::Scalar;
use crate::error::ErrorKind;
use crate::eval::{
    ArgVal, AstCache, EvalCtx, EvalFlags, FnRegistry, PassEnv, Reference, RuntimeValue, eval_expr,
    eval_formula_in, format_runtime,
};
use crate::graph::{CellCoord, DepGraph};
use crate::intern::ArrayPayload;
use crate::locale::LocaleId;
use crate::spill::{SpillRegion, SpillTable, blocks_spill};
use crate::storage::{CellFlags, CellSlot};
use crate::value::{Array2D, Value};
use crate::workbook::{CalcMode, Workbook};

/// Recalculation modes (F-3.6). Alias of workbook settings.
pub type RecalcMode = CalcMode;

/// Progress callback `(done, total, label)` used by the UI task runner.
pub type RecalcProgress = dyn Fn(u64, Option<u64>, &str) + Send + Sync;

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

    /// Hash an async call including its evaluated arguments.
    #[must_use]
    pub fn of_args(name: &str, args: &[ArgVal]) -> Self {
        let mut h = hash_bytes(0xcbf2_9ce4_8422_2325, name.as_bytes());
        for arg in args {
            h = hash_bytes(h, &[u8::from(arg.omitted)]);
            h = hash_runtime(h, &arg.value);
            h = hash_bytes(h, &[0xff]);
        }
        Self(h)
    }
}

fn hash_bytes(mut h: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        h = h.wrapping_mul(0x1000_0000_01b3) ^ u64::from(*byte);
    }
    h
}

fn hash_runtime(mut h: u64, value: &RuntimeValue) -> u64 {
    match value {
        RuntimeValue::Scalar(scalar) => hash_scalar(h, scalar),
        RuntimeValue::Array(array) => {
            h = hash_bytes(h, &[1]);
            h = hash_bytes(h, &array.rows.to_le_bytes());
            h = hash_bytes(h, &array.cols.to_le_bytes());
            for scalar in array.values.iter() {
                h = hash_scalar(h, scalar);
            }
            h
        }
        RuntimeValue::Lambda(lambda) => {
            h = hash_bytes(h, &[2]);
            for param in lambda.params.iter() {
                h = hash_bytes(h, param.name.as_bytes());
            }
            h = hash_bytes(h, format!("{:?}", lambda.body).as_bytes());
            for (name, captured) in lambda.closure.iter() {
                h = hash_bytes(h, name.as_bytes());
                h = hash_runtime(h, captured);
            }
            h
        }
        RuntimeValue::Ref(reference) => hash_reference(hash_bytes(h, &[3]), reference),
    }
}

fn hash_scalar(mut h: u64, scalar: &Scalar) -> u64 {
    match scalar {
        Scalar::Empty => hash_bytes(h, &[0]),
        Scalar::Number(number) => {
            h = hash_bytes(h, &[1]);
            hash_bytes(h, &number.to_bits().to_le_bytes())
        }
        Scalar::Bool(value) => hash_bytes(h, &[2, u8::from(*value)]),
        Scalar::Text(text) => {
            h = hash_bytes(h, &[3]);
            hash_bytes(h, text.as_bytes())
        }
        Scalar::Error(error) => {
            h = hash_bytes(h, &[4]);
            hash_bytes(h, error.as_str().as_bytes())
        }
    }
}

fn hash_reference(mut h: u64, reference: &Reference) -> u64 {
    match reference {
        Reference::Range {
            sheet,
            start_row,
            start_col,
            end_row,
            end_col,
        } => {
            h = hash_bytes(h, &[0]);
            h = hash_bytes(h, &sheet.index().to_le_bytes());
            h = hash_bytes(h, &start_row.to_le_bytes());
            h = hash_bytes(h, &start_col.to_le_bytes());
            h = hash_bytes(h, &end_row.to_le_bytes());
            hash_bytes(h, &end_col.to_le_bytes())
        }
        Reference::Union(parts) => {
            h = hash_bytes(h, &[1]);
            for part in parts {
                h = hash_reference(h, part);
            }
            h
        }
        Reference::ThreeD {
            sheets,
            start_row,
            start_col,
            end_row,
            end_col,
        } => {
            h = hash_bytes(h, &[2]);
            for sheet in sheets {
                h = hash_bytes(h, &sheet.index().to_le_bytes());
            }
            h = hash_bytes(h, &start_row.to_le_bytes());
            h = hash_bytes(h, &start_col.to_le_bytes());
            h = hash_bytes(h, &end_row.to_le_bytes());
            hash_bytes(h, &end_col.to_le_bytes())
        }
    }
}

/// Request passed to [`AsyncNodeProvider::evaluate`].
#[derive(Clone, Debug)]
pub struct AsyncRequest {
    /// Function name (`AI`, …).
    pub name: String,
    /// Formula cell.
    pub cell: CellCoord,
    /// Evaluated arguments, with references materialized to their current values.
    pub args: Vec<ArgVal>,
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
    /// Cooperative cancel restored the pre-pass workbook.
    pub cancelled: bool,
}

#[derive(Default)]
struct RecalcAccum {
    spill_blocked: Vec<(CellCoord, CellCoord)>,
    pending_async: Vec<CellCoord>,
    stale: Vec<CellCoord>,
    async_hints: Vec<(CellCoord, String)>,
    spill_follow: FxHashSet<CellCoord>,
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
    clock: Option<f64>,
    random_nonce: Option<u64>,
    locale: LocaleId,
    pass_env: PassEnv,
    async_provider: Option<Arc<dyn AsyncNodeProvider>>,
    /// Last dynamic-resolved refs per cell.
    dynamic_edges: FxHashMap<CellCoord, Vec<Reference>>,
    orphaned_spills: FxHashSet<CellCoord>,
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
            clock: None,
            random_nonce: None,
            locale: LocaleId::EN_US,
            pass_env: PassEnv::default(),
            async_provider: None,
            dynamic_edges: FxHashMap::default(),
            orphaned_spills: FxHashSet::default(),
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

    /// Inject a pass-stable clock serial (`NOW`/`TODAY`). `None` samples wall time.
    pub fn set_clock(&mut self, serial: Option<f64>) {
        self.clock = serial;
    }

    /// Inject the random nonce used by volatile random functions. `None` samples.
    pub fn set_random_nonce(&mut self, nonce: Option<u64>) {
        self.random_nonce = nonce;
    }

    /// Locale projected into [`PassEnv`] (not stored on workbook settings).
    pub fn set_locale(&mut self, locale: LocaleId) {
        self.locale = locale;
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
        self.refresh_registry_volatility(wb);
        self.dirty.clear();
        let formulas = self.graph.formula_cells();
        self.dirty.extend(formulas.iter().copied());
        let formula_set: FxHashSet<CellCoord> = formulas.into_iter().collect();
        self.orphaned_spills.extend(
            self.spill
                .origins()
                .filter(|origin| !formula_set.contains(origin)),
        );
        self.dynamic_edges.clear();
    }

    /// Record that `coord` changed (value or formula).
    pub fn notify_edit(&mut self, wb: &Workbook, coord: CellCoord) {
        if let Some(region) = self.spill.region_at(coord.sheet, coord.row, coord.col) {
            if region.origin == coord {
                let origin_still_has_formula = wb
                    .get(coord.sheet, coord.row, coord.col)
                    .ok()
                    .flatten()
                    .is_some_and(|slot| slot.formula.is_some());
                if !origin_still_has_formula {
                    self.orphaned_spills.insert(coord);
                }
            } else {
                self.dirty.insert(region.origin);
            }
        }
        let dependents = self.graph.dependents(coord);
        let was_formula = self.graph.calc_chain().contains(&coord);
        if let Ok(Some(slot)) = wb.get(coord.sheet, coord.row, coord.col) {
            if let Some(fid) = slot.formula {
                self.graph.upsert_formula(wb, &mut self.asts, coord, fid);
                self.refresh_cell_volatility(wb, coord);
            } else if was_formula {
                self.graph.remove_node(coord);
            }
        } else if was_formula {
            self.graph.remove_node(coord);
        }
        self.dirty.insert(coord);
        self.dirty.extend(dependents);
        // Dynamic cells that resolved to this cell last pass.
        let dyn_watchers: Vec<CellCoord> = self
            .dynamic_edges
            .iter()
            .filter_map(|(c, refs)| {
                if refs.iter().any(|r| reference_contains(r, coord)) {
                    Some(*c)
                } else {
                    None
                }
            })
            .collect();
        self.dirty.extend(dyn_watchers);
    }

    fn refresh_registry_volatility(&mut self, wb: &Workbook) {
        for cell in self.graph.formula_cells() {
            self.refresh_cell_volatility(wb, cell);
        }
    }

    fn refresh_cell_volatility(&mut self, wb: &Workbook, cell: CellCoord) {
        let Some(source) = wb
            .get(cell.sheet, cell.row, cell.col)
            .ok()
            .flatten()
            .and_then(|slot| slot.formula)
            .and_then(|fid| wb.intern().formulas.get(fid))
            .map(str::to_owned)
        else {
            return;
        };
        let Ok(formula) = self.asts.get_or_parse(&source) else {
            return;
        };
        let mut registry_volatile = false;
        formula.ast.walk(&mut |expr| {
            if let crate::formula::ExprKind::Call {
                callee: crate::formula::Callee::Name(name),
                ..
            } = &expr.kind
                && self.registry.lookup(name).is_some_and(|def| def.volatile)
            {
                registry_volatile = true;
            }
        });
        self.graph
            .set_volatile(cell, self.graph.is_volatile(cell) || registry_volatile);
    }

    /// Mark a completed async node for its second recalculation wave.
    pub fn notify_async_ready(&mut self, coord: CellCoord) {
        self.dirty.insert(coord);
    }

    /// Full recalc of every formula cell.
    pub fn recalc_full(&mut self, wb: &mut Workbook) -> RecalcResult {
        self.recalc_full_with_ctl(wb, None, None)
    }

    /// Full recalc with cooperative cancel. On cancel the workbook is restored.
    pub fn recalc_full_with_ctl(
        &mut self,
        wb: &mut Workbook,
        cancel: Option<&std::sync::atomic::AtomicBool>,
        progress: Option<std::sync::Arc<RecalcProgress>>,
    ) -> RecalcResult {
        self.rebuild(wb);
        self.run_ctl(wb, true, cancel, progress.as_deref())
    }

    /// Rebuild graph then full recalc.
    pub fn recalc_rebuild(&mut self, wb: &mut Workbook) -> RecalcResult {
        self.recalc_full(wb)
    }

    /// Rebuild with cooperative cancel.
    pub fn recalc_rebuild_with_ctl(
        &mut self,
        wb: &mut Workbook,
        cancel: Option<&std::sync::atomic::AtomicBool>,
        progress: Option<std::sync::Arc<RecalcProgress>>,
    ) -> RecalcResult {
        self.recalc_full_with_ctl(wb, cancel, progress)
    }

    /// Incremental recalc of the dirty set (no-op in manual mode).
    pub fn recalc_incremental(&mut self, wb: &mut Workbook) -> RecalcResult {
        self.recalc_incremental_with_ctl(wb, None, None)
    }

    /// Incremental recalc with cooperative cancel.
    pub fn recalc_incremental_with_ctl(
        &mut self,
        wb: &mut Workbook,
        cancel: Option<&std::sync::atomic::AtomicBool>,
        progress: Option<std::sync::Arc<RecalcProgress>>,
    ) -> RecalcResult {
        match wb.settings().calc_mode {
            CalcMode::Manual => RecalcResult::default(),
            CalcMode::Automatic | CalcMode::AutomaticExceptTables => {
                if self.graph.is_empty() {
                    self.rebuild(wb);
                }
                for v in self.graph.volatiles() {
                    self.dirty.insert(v);
                }
                self.run_ctl(wb, false, cancel, progress.as_deref())
            }
        }
    }

    fn sample_pass_env(&self) -> PassEnv {
        PassEnv {
            clock: self.clock.unwrap_or_else(wall_clock_serial),
            locale: self.locale,
            random_nonce: self.random_nonce.unwrap_or_else(fresh_nonce),
        }
    }

    fn run_ctl(
        &mut self,
        wb: &mut Workbook,
        full: bool,
        cancel: Option<&std::sync::atomic::AtomicBool>,
        progress: Option<&RecalcProgress>,
    ) -> RecalcResult {
        let backup = cancel.map(|_| wb.clone());
        let t0 = Instant::now();
        self.pass = self.pass.saturating_add(1);
        self.pass_env = self.sample_pass_env();
        let undo = wb.undo_log_mut().is_enabled();
        wb.undo_log_mut().set_enabled(false);
        for origin in self.orphaned_spills.drain() {
            if let Some(region) = self.spill.get(origin) {
                extend_spill_dependents(&self.graph, region, &mut self.dirty);
            }
            self.spill.clear_ghosts(wb, origin);
            self.spill.remove(origin);
        }
        wb.undo_log_mut().set_enabled(undo);
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

        // Cached formula values are derived state, not user edits. Keep every
        // result commit (including circular/iterative paths) out of undo.
        let recalc_undo = wb.undo_log_mut().is_enabled();
        wb.undo_log_mut().set_enabled(false);
        let mut circular = self.graph.circular_set(&dirty);
        let iteration = wb.settings().iteration;
        let mut evaluated = 0u64;
        let mut accum = RecalcAccum::default();

        if !circular.is_empty() && !iteration.enabled {
            for c in &circular {
                commit_scalar(wb, *c, Scalar::Number(0.0), CellFlags::DEFAULT);
            }
            evaluated += circular.len() as u64;
        }

        let gens = self.graph.generations(&dirty);
        let total = gens.iter().map(Vec::len).sum::<usize>() as u64;
        for generation in gens {
            if cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst)) {
                if let Some(backup) = backup {
                    *wb = backup;
                    self.rebuild(wb);
                }
                return RecalcResult {
                    cells_evaluated: evaluated,
                    elapsed_ms: u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX),
                    cancelled: true,
                    ..RecalcResult::default()
                };
            }
            evaluated += self.eval_generation(wb, &generation, &mut accum) as u64;
            if let Some(progress) = progress {
                progress(evaluated, Some(total.max(1)), "recalc");
            }
        }

        let circular_nodes = circular.clone();
        if iteration.enabled && !circular.is_empty() {
            evaluated += self.iterate_cycle(
                wb,
                &circular,
                iteration.max_iterations,
                iteration.max_change,
            ) as u64;
            accum.spill_follow.extend(circular.iter().copied());
            circular.clear();
        }

        // A direct reference to a spill ghost is statically an edge to that
        // cell, not to the spill origin. Re-run affected formulas after the
        // origin commits, and likewise re-run dependents after an iterative
        // cycle has converged. Chained spills may require several bounded waves.
        let circular_set: FxHashSet<CellCoord> = circular_nodes.into_iter().collect();
        let mut waves_left = self.graph.formula_cells().len().max(1);
        while !accum.spill_follow.is_empty() && waves_left > 0 {
            waves_left -= 1;
            let seeds: Vec<CellCoord> = accum.spill_follow.drain().collect();
            let mut wave = self.graph.propagate(seeds);
            wave.retain(|cell| !circular_set.contains(cell));
            if wave.is_empty() {
                break;
            }
            let mut wave_accum = RecalcAccum::default();
            for generation in self.graph.generations(&wave) {
                evaluated += self.eval_generation(wb, &generation, &mut wave_accum) as u64;
            }
            accum.spill_blocked.extend(wave_accum.spill_blocked);
            accum.pending_async.extend(wave_accum.pending_async);
            accum.stale.extend(wave_accum.stale);
            accum.async_hints.extend(wave_accum.async_hints);
            accum.spill_follow = wave_accum.spill_follow;
        }

        accum.pending_async.sort_unstable();
        accum.pending_async.dedup();
        accum
            .stale
            .extend(self.graph.propagate(accum.pending_async.iter().copied()));
        accum.stale.sort_unstable();
        accum.stale.dedup();
        let undo = wb.undo_log_mut().is_enabled();
        wb.undo_log_mut().set_enabled(false);
        for cell in &accum.stale {
            set_stale_flag(wb, *cell, true);
        }
        wb.undo_log_mut().set_enabled(undo);

        // Dynamic nodes: record resolved refs for the next dirty pass.
        // (Evaluated as part of generations when they have no static preds.)

        wb.undo_log_mut().set_enabled(recalc_undo);
        RecalcResult {
            cells_evaluated: evaluated,
            elapsed_ms: t0.elapsed().as_millis() as u64,
            circular,
            spill_blocked: accum.spill_blocked,
            pending_async: accum.pending_async,
            stale: accum.stale,
            async_hints: accum.async_hints,
            cancelled: false,
        }
    }

    fn eval_generation(
        &mut self,
        wb: &mut Workbook,
        generation: &[CellCoord],
        accum: &mut RecalcAccum,
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
            let env = self.pass_env;
            let provider = self.async_provider.clone();
            let eval_one = |cell: CellCoord| {
                eval_one_cell(
                    wb_ref,
                    registry,
                    spill,
                    asts,
                    pass,
                    env,
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
                accum.pending_async.push(cell);
            }
            if flags.stale {
                accum.stale.push(cell);
            }
            if let Some(h) = flags.hint {
                accum.async_hints.push((cell, h));
            }
            if flags.dynamic.is_empty() {
                self.dynamic_edges.remove(&cell);
            } else {
                let mut refs = Vec::new();
                for reference in flags.dynamic {
                    if !refs.contains(&reference) {
                        refs.push(reference);
                    }
                }
                self.dynamic_edges.insert(cell, refs);
            }
            let old_spill = self.spill.get(cell);
            let blocked = commit_value(wb, &mut self.spill, cell, value, cse, flags.stale);
            if let Some(region) = old_spill {
                extend_spill_dependents(&self.graph, region, &mut accum.spill_follow);
            }
            if let Some(region) = self.spill.get(cell) {
                extend_spill_dependents(&self.graph, region, &mut accum.spill_follow);
            }
            if let Some(block) = blocked {
                accum.spill_blocked.push((cell, block));
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
                let (value, _) = eval_formula_in(
                    wb,
                    &self.registry,
                    &self.spill,
                    cell,
                    &formula.ast,
                    self.pass,
                    self.pass_env,
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

#[allow(clippy::too_many_arguments)]
fn eval_one_cell(
    wb: &Workbook,
    registry: &FnRegistry,
    spill: &SpillTable,
    asts: &AstCache,
    pass: u32,
    env: PassEnv,
    provider: Option<&dyn AsyncNodeProvider>,
    cell: CellCoord,
) -> Option<(CellCoord, RuntimeValue, EvalFlags, bool)> {
    let slot = wb.get(cell.sheet, cell.row, cell.col).ok()??;
    let fid = slot.formula?;
    let src = wb.intern().formulas.get(fid)?;
    let cse = slot.flags.array();
    let formula = asts.peek(src)?;
    let mut ctx = EvalCtx::new(wb, registry, spill, cell, pass)
        .with_pass_env(env)
        .with_async_provider(provider);
    let raw = eval_expr(&mut ctx, &formula.ast);
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

fn reference_contains(r: &Reference, cell: CellCoord) -> bool {
    match r {
        Reference::Range {
            sheet,
            start_row,
            start_col,
            end_row,
            end_col,
        } => {
            cell.sheet == *sheet
                && cell.row >= (*start_row).min(*end_row)
                && cell.row <= (*start_row).max(*end_row)
                && cell.col >= (*start_col).min(*end_col)
                && cell.col <= (*start_col).max(*end_col)
        }
        Reference::Union(parts) => parts.iter().any(|part| reference_contains(part, cell)),
        Reference::ThreeD {
            sheets,
            start_row,
            start_col,
            end_row,
            end_col,
        } => {
            sheets.contains(&cell.sheet)
                && cell.row >= (*start_row).min(*end_row)
                && cell.row <= (*start_row).max(*end_row)
                && cell.col >= (*start_col).min(*end_col)
                && cell.col <= (*start_col).max(*end_col)
        }
    }
}

fn extend_spill_dependents(graph: &DepGraph, region: SpillRegion, out: &mut FxHashSet<CellCoord>) {
    if region.blocked_by.is_some() {
        return;
    }
    for dr in 0..region.rows {
        for dc in 0..region.cols {
            if dr == 0 && dc == 0 {
                continue;
            }
            let ghost = CellCoord::new(
                region.origin.sheet,
                region.origin.row.saturating_add(dr),
                region.origin.col.saturating_add(dc as u16),
            );
            out.extend(graph.dependents(ghost));
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
        RuntimeValue::Array(a) => match a.validate() {
            Err(error) => {
                commit_scalar(wb, cell, Scalar::Error(error), flags(stale));
                None
            }
            Ok(_) if cse || (a.rows == 1 && a.cols == 1) => {
                let s = a.values.first().cloned().unwrap_or(Scalar::Empty);
                commit_scalar(wb, cell, s, flags(stale).with(CellFlags::ARRAY, cse));
                None
            }
            Ok(_) => spill_array(wb, spill, cell, &a, stale),
        },
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
    let mut slot = wb
        .get(sheet, row, col)
        .ok()
        .flatten()
        .copied()
        .unwrap_or_else(CellSlot::empty);
    slot.value = value;
    slot.formula = None;
    slot.flags = slot
        .flags
        .with(CellFlags::DIRTY, false)
        .with(CellFlags::SPILL, true)
        .with(CellFlags::ARRAY, false)
        .with(CellFlags::STALE, false);
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
    slot.flags = slot
        .flags
        .with(CellFlags::DIRTY, false)
        .with(CellFlags::ARRAY, slot.flags.array() || flags.array())
        .with(CellFlags::SPILL, false)
        .with(CellFlags::STALE, flags.stale());
    let _ = wb.set_slot(cell.sheet, cell.row, cell.col, slot);
    release_intern_extra(wb, value);
}

fn set_stale_flag(wb: &mut Workbook, cell: CellCoord, stale: bool) {
    let Ok(Some(slot)) = wb.get(cell.sheet, cell.row, cell.col) else {
        return;
    };
    let mut slot = *slot;
    slot.flags = slot.flags.with(CellFlags::STALE, stale);
    let _ = wb.set_slot(cell.sheet, cell.row, cell.col, slot);
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

fn wall_clock_serial() -> f64 {
    // 1 January 1970 in the 1900 date system (Excel).
    const UNIX_EPOCH_SERIAL_1900: f64 = 25_569.0;
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => UNIX_EPOCH_SERIAL_1900 + duration.as_secs_f64() / 86_400.0,
        Err(_) => 0.0,
    }
}

fn fresh_nonce() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64 ^ duration.as_secs())
        .unwrap_or(0xA5A5_A5A5_A5A5_A5A5)
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
