//! Range-aware dependency graph (spec §11.3, F-3.6).
//!
//! Single-cell refs are direct edges. Range refs go through per-sheet buckets
//! (whole sheet / whole row / whole column / 256×256 blocks) so `A:A` is one
//! column-bucket edge.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::addr::{CellRef, RangeRef, SheetId, SheetSpec};
use crate::eval::{AstCache, Reference};
use crate::formula::{Deps, collect_deps};
use crate::intern::FormulaId;
use crate::limits::{MAX_COLS, MAX_ROWS};
use crate::names::{MAX_DEFINED_NAME_DEPTH, NameScope};
use crate::storage::BLOCK_SIZE;
use crate::workbook::Workbook;

/// Formula-cell coordinate, ordered by `(sheet, row, col)` for determinism.
///
/// ```
/// use omacell_core::addr::SheetId;
/// use omacell_core::graph::CellCoord;
/// let a = CellCoord::new(SheetId::new(0), 0, 0);
/// let b = CellCoord::new(SheetId::new(0), 0, 1);
/// assert!(a < b);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellCoord {
    /// Sheet.
    pub sheet: SheetId,
    /// 0-based row.
    pub row: u32,
    /// 0-based column.
    pub col: u16,
}

impl CellCoord {
    /// Construct a coordinate.
    #[must_use]
    pub fn new(sheet: SheetId, row: u32, col: u16) -> Self {
        Self { sheet, row, col }
    }
}

/// One precedent of a formula cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Precedent {
    /// A single cell.
    Cell(CellCoord),
    /// A rectangle on one sheet.
    Range {
        /// Sheet.
        sheet: SheetId,
        /// Range body.
        range: RangeRef,
        /// Whole-column bucket (O(1) `A:A`).
        whole_col: bool,
        /// Whole-row bucket.
        whole_row: bool,
    },
    /// 3-D rectangle.
    ThreeD {
        /// Sheets in workbook order.
        sheets: Vec<SheetId>,
        /// Range body on each sheet.
        range: RangeRef,
    },
}

#[derive(Clone, Debug, Default)]
struct Node {
    volatile: bool,
    dynamic: bool,
    precedents: Vec<Precedent>,
    static_precedents: usize,
    /// Formula cells that directly depend on this cell (not via a range bucket).
    cell_dependents: Vec<CellCoord>,
}

#[derive(Clone, Debug, Default)]
struct SheetBuckets {
    sheet: Vec<CellCoord>,
    cols: FxHashMap<u16, Vec<CellCoord>>,
    rows: FxHashMap<u32, Vec<CellCoord>>,
    blocks: FxHashMap<(u32, u16), Vec<(CellCoord, RangeRef)>>,
}

/// Per-workbook dependency graph.
#[derive(Clone, Debug, Default)]
pub struct DepGraph {
    nodes: FxHashMap<CellCoord, Node>,
    buckets: FxHashMap<SheetId, SheetBuckets>,
    /// Sorted formula cells (calc chain).
    chain: Vec<CellCoord>,
}

impl DepGraph {
    /// Empty graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether any formula nodes exist.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Formula cells in deterministic order (the persisted calc chain).
    #[must_use]
    pub fn calc_chain(&self) -> &[CellCoord] {
        &self.chain
    }

    /// All formula cells.
    #[must_use]
    pub fn formula_cells(&self) -> Vec<CellCoord> {
        self.chain.clone()
    }

    /// Direct precedents of a formula cell.
    #[must_use]
    pub fn precedents(&self, cell: CellCoord) -> &[Precedent] {
        self.nodes
            .get(&cell)
            .map(|n| n.precedents.as_slice())
            .unwrap_or(&[])
    }

    /// Whether this formula is volatile.
    #[must_use]
    pub fn is_volatile(&self, cell: CellCoord) -> bool {
        self.nodes.get(&cell).map(|n| n.volatile).unwrap_or(false)
    }

    pub(crate) fn set_volatile(&mut self, cell: CellCoord, volatile: bool) {
        if let Some(node) = self.nodes.get_mut(&cell) {
            node.volatile = volatile;
        }
    }

    pub(crate) fn replace_dynamic_precedents(
        &mut self,
        updates: &[(CellCoord, Vec<Reference>)],
    ) -> Vec<CellCoord> {
        let mut changed = Vec::new();
        for (cell, references) in updates {
            let precedents = precedents_from_references(references);
            let Some(node) = self.nodes.get_mut(cell) else {
                continue;
            };
            let static_len = node.static_precedents.min(node.precedents.len());
            if node.precedents[static_len..] == precedents {
                continue;
            }
            node.precedents.truncate(static_len);
            node.precedents.extend(precedents);
            changed.push(*cell);
        }
        if !changed.is_empty() {
            changed.sort_unstable();
            self.finish_edges();
        }
        changed
    }

    /// Whether this formula contains `INDIRECT`/`OFFSET`.
    #[must_use]
    pub fn is_dynamic(&self, cell: CellCoord) -> bool {
        self.nodes.get(&cell).map(|n| n.dynamic).unwrap_or(false)
    }

    /// Volatile formula cells (sorted).
    #[must_use]
    pub fn volatiles(&self) -> Vec<CellCoord> {
        let mut v: Vec<CellCoord> = self
            .nodes
            .iter()
            .filter(|(_, n)| n.volatile)
            .map(|(c, _)| *c)
            .collect();
        v.sort_unstable();
        v
    }

    /// Dynamic formula cells (sorted).
    #[must_use]
    pub fn dynamics(&self) -> Vec<CellCoord> {
        let mut v: Vec<CellCoord> = self
            .nodes
            .iter()
            .filter(|(_, n)| n.dynamic)
            .map(|(c, _)| *c)
            .collect();
        v.sort_unstable();
        v
    }

    /// Dependents of a cell (direct + range buckets), sorted and unique.
    #[must_use]
    pub fn dependents(&self, cell: CellCoord) -> Vec<CellCoord> {
        let mut out: FxHashSet<CellCoord> = FxHashSet::default();
        if let Some(n) = self.nodes.get(&cell) {
            out.extend(n.cell_dependents.iter().copied());
        }
        if let Some(b) = self.buckets.get(&cell.sheet) {
            out.extend(b.sheet.iter().copied());
            if let Some(v) = b.cols.get(&cell.col) {
                out.extend(v.iter().copied());
            }
            if let Some(v) = b.rows.get(&cell.row) {
                out.extend(v.iter().copied());
            }
            let br = cell.row / BLOCK_SIZE;
            let bc = u32::from(cell.col) / BLOCK_SIZE;
            if let Some(v) = b.blocks.get(&(br, bc as u16)) {
                for (dep, range) in v {
                    if range_contains(*range, cell.row, cell.col) {
                        out.insert(*dep);
                    }
                }
            }
        }
        let mut list: Vec<CellCoord> = out.into_iter().collect();
        list.sort_unstable();
        list
    }

    /// Rebuild from every formula cell in `wb`.
    pub fn rebuild(&mut self, wb: &Workbook, asts: &mut AstCache) {
        *self = Self::new();
        let mut cells = Vec::new();
        for sheet in wb.sheets() {
            for (row, col, slot) in sheet.store.iter() {
                if let Some(fid) = slot.formula {
                    cells.push((CellCoord::new(sheet.id, row, col), fid));
                }
            }
        }
        cells.sort_by_key(|(c, _)| *c);
        for (coord, fid) in &cells {
            self.add_node(wb, asts, *coord, *fid);
        }
        self.chain = cells.into_iter().map(|(c, _)| c).collect();
        self.finish_edges();
    }

    /// Insert or replace one formula node (incremental edit).
    pub fn upsert_formula(
        &mut self,
        wb: &Workbook,
        asts: &mut AstCache,
        coord: CellCoord,
        fid: FormulaId,
    ) {
        self.remove_node(coord);
        self.add_node(wb, asts, coord, fid);
        if !self.chain.contains(&coord) {
            self.chain.push(coord);
            self.chain.sort_unstable();
        }
        self.finish_edges();
    }

    /// Remove a formula node.
    pub fn remove_node(&mut self, coord: CellCoord) {
        let mut cell_dependents = self
            .nodes
            .remove(&coord)
            .map(|node| node.cell_dependents)
            .unwrap_or_default();
        cell_dependents.retain(|dependent| *dependent != coord);
        self.chain.retain(|c| *c != coord);
        for b in self.buckets.values_mut() {
            b.sheet.retain(|c| *c != coord);
            for v in b.cols.values_mut() {
                v.retain(|c| *c != coord);
            }
            for v in b.rows.values_mut() {
                v.retain(|c| *c != coord);
            }
            for v in b.blocks.values_mut() {
                v.retain(|(c, _)| *c != coord);
            }
        }
        for n in self.nodes.values_mut() {
            n.cell_dependents.retain(|c| *c != coord);
        }
        if !cell_dependents.is_empty() {
            self.nodes.entry(coord).or_default().cell_dependents = cell_dependents;
        }
    }

    fn add_node(&mut self, wb: &Workbook, asts: &mut AstCache, coord: CellCoord, fid: FormulaId) {
        let Some(src) = wb.intern().formulas.get(fid) else {
            return;
        };
        let Ok(ref formula) = asts.get_or_parse(src) else {
            self.nodes.insert(
                coord,
                Node {
                    volatile: false,
                    dynamic: false,
                    precedents: Vec::new(),
                    static_precedents: 0,
                    cell_dependents: Vec::new(),
                },
            );
            return;
        };
        let deps = collect_deps(&formula.ast);
        let precedents = resolve_deps(wb, coord.sheet, &deps);
        let static_precedents = precedents.len();
        self.nodes.insert(
            coord,
            Node {
                volatile: deps.volatile,
                dynamic: deps.dynamic,
                precedents,
                static_precedents,
                cell_dependents: Vec::new(),
            },
        );
    }

    fn finish_edges(&mut self) {
        // Reset reverse edges / buckets.
        for n in self.nodes.values_mut() {
            n.cell_dependents.clear();
        }
        self.buckets.clear();
        let coords: Vec<CellCoord> = self.nodes.keys().copied().collect();
        for coord in coords {
            let precs = self
                .nodes
                .get(&coord)
                .map(|n| n.precedents.clone())
                .unwrap_or_default();
            for p in precs {
                self.add_reverse(coord, &p);
            }
        }
        for n in self.nodes.values_mut() {
            n.cell_dependents.sort_unstable();
            n.cell_dependents.dedup();
        }
    }

    fn add_reverse(&mut self, dep: CellCoord, p: &Precedent) {
        match p {
            Precedent::Cell(c) => {
                if let Some(n) = self.nodes.get_mut(c) {
                    n.cell_dependents.push(dep);
                } else {
                    // Precedent is a literal / empty cell: still need a reverse
                    // slot so dirty-from-edit finds `dep`. Store on a phantom node.
                    self.nodes.entry(*c).or_default().cell_dependents.push(dep);
                }
            }
            Precedent::Range {
                sheet,
                range,
                whole_col,
                whole_row,
            } => {
                let b = self.buckets.entry(*sheet).or_default();
                if *whole_col && range.start.col == range.end.col {
                    b.cols.entry(range.start.col).or_default().push(dep);
                } else if *whole_row && range.start.row == range.end.row {
                    b.rows.entry(range.start.row).or_default().push(dep);
                } else if *whole_col && *whole_row {
                    b.sheet.push(dep);
                } else {
                    for block in blocks_of(*range) {
                        b.blocks.entry(block).or_default().push((dep, *range));
                    }
                }
            }
            Precedent::ThreeD { sheets, range } => {
                for sheet in sheets {
                    self.add_reverse(
                        dep,
                        &Precedent::Range {
                            sheet: *sheet,
                            range: *range,
                            whole_col: range.whole_col,
                            whole_row: range.whole_row,
                        },
                    );
                }
            }
        }
    }

    /// Close `seeds` under reverse edges. Result is sorted.
    #[must_use]
    pub fn propagate(&self, seeds: impl IntoIterator<Item = CellCoord>) -> Vec<CellCoord> {
        let mut out: FxHashSet<CellCoord> = FxHashSet::default();
        let mut stack: Vec<CellCoord> = seeds.into_iter().collect();
        while let Some(c) = stack.pop() {
            if !out.insert(c) {
                continue;
            }
            for d in self.dependents(c) {
                if !out.contains(&d) {
                    stack.push(d);
                }
            }
        }
        // Keep only formula cells (phantoms for literals have empty precedents
        // and no formula — they still sit in `nodes`. Filter to chain.)
        let chain: FxHashSet<CellCoord> = self.chain.iter().copied().collect();
        let mut list: Vec<CellCoord> = out.into_iter().filter(|c| chain.contains(c)).collect();
        list.sort_unstable();
        list
    }

    /// Cells in strongly connected components with size > 1 or a self-loop.
    #[must_use]
    pub fn circular_set(&self, cells: &[CellCoord]) -> Vec<CellCoord> {
        let mut circular: Vec<_> = self
            .circular_components(cells)
            .into_iter()
            .flatten()
            .collect();
        circular.sort_unstable();
        circular
    }

    /// Strongly connected components among `cells` with size > 1 or a self-loop.
    /// Components and their cells are returned in deterministic coordinate order.
    #[must_use]
    pub fn circular_components(&self, cells: &[CellCoord]) -> Vec<Vec<CellCoord>> {
        let mut vertices = cells.to_vec();
        vertices.sort_unstable();
        vertices.dedup();
        let all: FxHashSet<CellCoord> = vertices.iter().copied().collect();

        // Most production graphs are acyclic. Peel every topologically ordered
        // node first so the exact SCC pass below only allocates for the small
        // residue containing cycles and their downstream cells.
        let mut indegree: FxHashMap<CellCoord, usize> = FxHashMap::default();
        for &cell in &vertices {
            indegree.insert(cell, self.precedents_in(cell, &all).len());
        }
        let mut queue: Vec<CellCoord> = indegree
            .iter()
            .filter_map(|(cell, degree)| (*degree == 0).then_some(*cell))
            .collect();
        queue.sort_unstable();
        let mut cursor = 0usize;
        while cursor < queue.len() {
            let cell = queue[cursor];
            cursor += 1;
            if indegree.remove(&cell).is_none() {
                continue;
            }
            for dependent in self.dependents(cell) {
                if let Some(degree) = indegree.get_mut(&dependent) {
                    *degree = degree.saturating_sub(1);
                    if *degree == 0 {
                        queue.push(dependent);
                    }
                }
            }
        }
        if indegree.is_empty() {
            return Vec::new();
        }
        vertices = indegree.into_keys().collect();
        vertices.sort_unstable();
        let among: FxHashSet<CellCoord> = vertices.iter().copied().collect();

        // Build both directions once, then use iterative Kosaraju so a long
        // formula chain cannot overflow the Rust call stack. Kahn leftovers are
        // insufficient here: they also contain acyclic cells downstream of a
        // real cycle.
        let mut forward: FxHashMap<CellCoord, Vec<CellCoord>> = FxHashMap::default();
        let mut reverse: FxHashMap<CellCoord, Vec<CellCoord>> = FxHashMap::default();
        for &cell in &vertices {
            reverse.entry(cell).or_default();
            let precedents = self.precedents_in(cell, &among);
            for precedent in &precedents {
                reverse.entry(*precedent).or_default().push(cell);
            }
            forward.insert(cell, precedents);
        }
        for edges in reverse.values_mut() {
            edges.sort_unstable();
            edges.dedup();
        }

        let mut seen: FxHashSet<CellCoord> = FxHashSet::default();
        let mut finish = Vec::with_capacity(vertices.len());
        for &start in &vertices {
            if !seen.insert(start) {
                continue;
            }
            let mut stack = vec![(start, false)];
            while let Some((cell, expanded)) = stack.pop() {
                if expanded {
                    finish.push(cell);
                    continue;
                }
                stack.push((cell, true));
                if let Some(edges) = forward.get(&cell) {
                    for &next in edges.iter().rev() {
                        if seen.insert(next) {
                            stack.push((next, false));
                        }
                    }
                }
            }
        }

        seen.clear();
        let mut circular = Vec::new();
        for &start in finish.iter().rev() {
            if !seen.insert(start) {
                continue;
            }
            let mut component = Vec::new();
            let mut stack = vec![start];
            while let Some(cell) = stack.pop() {
                component.push(cell);
                if let Some(edges) = reverse.get(&cell) {
                    for &next in edges.iter().rev() {
                        if seen.insert(next) {
                            stack.push(next);
                        }
                    }
                }
            }
            let self_loop = component.len() == 1
                && forward
                    .get(&component[0])
                    .is_some_and(|edges| edges.contains(&component[0]));
            if component.len() > 1 || self_loop {
                component.sort_unstable();
                circular.push(component);
            }
        }
        circular.sort_by_key(|component| component.first().copied());
        circular
    }

    fn precedents_in(&self, c: CellCoord, among: &FxHashSet<CellCoord>) -> Vec<CellCoord> {
        let mut out = Vec::new();
        let Some(n) = self.nodes.get(&c) else {
            return out;
        };
        for p in &n.precedents {
            match p {
                Precedent::Cell(x) => {
                    if among.contains(x) {
                        out.push(*x);
                    }
                }
                Precedent::Range { sheet, range, .. } => {
                    for other in among {
                        if other.sheet == *sheet && range_contains(*range, other.row, other.col) {
                            out.push(*other);
                        }
                    }
                }
                Precedent::ThreeD { sheets, range } => {
                    for other in among {
                        if sheets.contains(&other.sheet)
                            && range_contains(*range, other.row, other.col)
                        {
                            out.push(*other);
                        }
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Topological generations of `cells`. Circular cells are omitted (caller
    /// handles them). Within a generation, cells are sorted.
    #[must_use]
    pub fn generations(&self, cells: &[CellCoord]) -> Vec<Vec<CellCoord>> {
        let among: FxHashSet<CellCoord> = cells.iter().copied().collect();
        let circ: FxHashSet<CellCoord> = self.circular_set(cells).into_iter().collect();
        let acyclic: Vec<CellCoord> = cells
            .iter()
            .copied()
            .filter(|c| !circ.contains(c))
            .collect();
        let set: FxHashSet<CellCoord> = acyclic.iter().copied().collect();
        let mut indeg: FxHashMap<CellCoord, usize> = FxHashMap::default();
        for &c in &acyclic {
            let n = self
                .precedents_in(c, &among)
                .into_iter()
                .filter(|p| set.contains(p))
                .count();
            indeg.insert(c, n);
        }
        let mut gens = Vec::new();
        let mut remaining: FxHashSet<CellCoord> = set.clone();
        while !remaining.is_empty() {
            let mut generation: Vec<CellCoord> = remaining
                .iter()
                .copied()
                .filter(|c| indeg.get(c).copied().unwrap_or(0) == 0)
                .collect();
            if generation.is_empty() {
                // Should be circular leftovers; stop.
                break;
            }
            generation.sort_unstable();
            for c in &generation {
                remaining.remove(c);
            }
            for c in &generation {
                for d in self.dependents(*c) {
                    if remaining.contains(&d)
                        && let Some(n) = indeg.get_mut(&d)
                    {
                        *n = n.saturating_sub(1);
                    }
                }
            }
            gens.push(generation);
        }
        gens
    }
}

fn range_contains(range: RangeRef, row: u32, col: u16) -> bool {
    let r1 = range.start.row.min(range.end.row);
    let r2 = range.start.row.max(range.end.row);
    let c1 = range.start.col.min(range.end.col);
    let c2 = range.start.col.max(range.end.col);
    row >= r1 && row <= r2 && col >= c1 && col <= c2
}

fn precedents_from_references(references: &[Reference]) -> Vec<Precedent> {
    let mut precedents = Vec::new();
    for reference in references {
        push_reference_precedents(reference, &mut precedents);
    }
    precedents
}

fn push_reference_precedents(reference: &Reference, precedents: &mut Vec<Precedent>) {
    match reference {
        Reference::Range {
            sheet,
            start_row,
            start_col,
            end_row,
            end_col,
        } => {
            let (start_row, end_row) = ((*start_row).min(*end_row), (*start_row).max(*end_row));
            let (start_col, end_col) = ((*start_col).min(*end_col), (*start_col).max(*end_col));
            if start_row == end_row && start_col == end_col {
                push_precedent(
                    precedents,
                    Precedent::Cell(CellCoord::new(*sheet, start_row, start_col)),
                );
                return;
            }
            let range = reference_range(start_row, start_col, end_row, end_col);
            push_precedent(
                precedents,
                Precedent::Range {
                    sheet: *sheet,
                    range,
                    whole_col: start_row == 0 && end_row == MAX_ROWS.saturating_sub(1),
                    whole_row: start_col == 0 && end_col == MAX_COLS.saturating_sub(1),
                },
            );
        }
        Reference::Union(parts) => {
            for part in parts {
                push_reference_precedents(part, precedents);
            }
        }
        Reference::ThreeD {
            sheets,
            start_row,
            start_col,
            end_row,
            end_col,
        } => {
            if sheets.is_empty() {
                return;
            }
            let range = reference_range(
                (*start_row).min(*end_row),
                (*start_col).min(*end_col),
                (*start_row).max(*end_row),
                (*start_col).max(*end_col),
            );
            push_precedent(
                precedents,
                Precedent::ThreeD {
                    sheets: sheets.clone(),
                    range,
                },
            );
        }
    }
}

fn reference_range(start_row: u32, start_col: u16, end_row: u32, end_col: u16) -> RangeRef {
    let mut range = RangeRef::from_corners(
        CellRef {
            sheet: None,
            row: start_row,
            col: start_col,
            row_abs: true,
            col_abs: true,
        },
        CellRef {
            sheet: None,
            row: end_row,
            col: end_col,
            row_abs: true,
            col_abs: true,
        },
    );
    range.whole_col = start_row == 0 && end_row == MAX_ROWS.saturating_sub(1);
    range.whole_row = start_col == 0 && end_col == MAX_COLS.saturating_sub(1);
    range
}

fn push_precedent(precedents: &mut Vec<Precedent>, precedent: Precedent) {
    if !precedents.contains(&precedent) {
        precedents.push(precedent);
    }
}

fn blocks_of(range: RangeRef) -> Vec<(u32, u16)> {
    let r1 = range.start.row.min(range.end.row) / BLOCK_SIZE;
    let r2 = range.start.row.max(range.end.row) / BLOCK_SIZE;
    let c1 = u32::from(range.start.col.min(range.end.col)) / BLOCK_SIZE;
    let c2 = u32::from(range.start.col.max(range.end.col)) / BLOCK_SIZE;
    let mut out = Vec::new();
    for r in r1..=r2 {
        for c in c1..=c2 {
            out.push((r, c as u16));
        }
    }
    out
}

fn resolve_deps(wb: &Workbook, default_sheet: SheetId, deps: &Deps) -> Vec<Precedent> {
    resolve_deps_inner(wb, default_sheet, deps, &mut FxHashSet::default())
}

fn resolve_deps_inner(
    wb: &Workbook,
    default_sheet: SheetId,
    deps: &Deps,
    active_names: &mut FxHashSet<(NameScope, String)>,
) -> Vec<Precedent> {
    let mut out = Vec::new();
    for (spec, range) in &deps.ranges {
        match resolve_sheet_spec(wb, spec.as_ref(), default_sheet) {
            SheetResolve::One(sheet) => {
                let whole_col = range.whole_col
                    || (range.start.row == 0 && range.end.row == MAX_ROWS.saturating_sub(1));
                let whole_row = range.whole_row
                    || (range.start.col == 0 && range.end.col == MAX_COLS.saturating_sub(1));
                if range.start.row == range.end.row
                    && range.start.col == range.end.col
                    && !whole_col
                    && !whole_row
                {
                    out.push(Precedent::Cell(CellCoord::new(
                        sheet,
                        range.start.row,
                        range.start.col,
                    )));
                } else {
                    out.push(Precedent::Range {
                        sheet,
                        range: *range,
                        whole_col,
                        whole_row,
                    });
                }
                push_cse_anchors(wb, sheet, *range, &mut out);
            }
            SheetResolve::Span(sheets) => {
                for sheet in &sheets {
                    push_cse_anchors(wb, *sheet, *range, &mut out);
                }
                out.push(Precedent::ThreeD {
                    sheets,
                    range: *range,
                });
            }
            SheetResolve::Missing => {}
        }
    }
    for (spec, name) in &deps.names {
        let sheet = match spec {
            Some(s) => wb.resolve_sheet_name(&s.start).unwrap_or(default_sheet),
            None => default_sheet,
        };
        if let Some(n) = wb.names().resolve(sheet, name) {
            match &n.referent {
                crate::names::NameReferent::Range(r) => {
                    let sh = r.start.sheet.unwrap_or(sheet);
                    out.push(Precedent::Range {
                        sheet: sh,
                        range: *r,
                        whole_col: r.whole_col,
                        whole_row: r.whole_row,
                    });
                    push_cse_anchors(wb, sh, *r, &mut out);
                }
                crate::names::NameReferent::Formula(src) => {
                    if active_names.len() >= MAX_DEFINED_NAME_DEPTH {
                        continue;
                    }
                    let key = (n.scope, n.name.to_lowercase());
                    if !active_names.insert(key.clone()) {
                        continue;
                    }
                    if let Ok(f) = crate::formula::parse(src) {
                        let inner = collect_deps(&f.ast);
                        out.extend(resolve_deps_inner(wb, sheet, &inner, active_names));
                    }
                    active_names.remove(&key);
                }
                crate::names::NameReferent::Constant(_) => {}
            }
        }
    }
    for tname in &deps.tables {
        if let Some(t) = wb.tables().get_by_name(tname) {
            let start = crate::addr::CellRef {
                sheet: Some(t.sheet),
                row: t.start_row,
                col: t.start_col,
                row_abs: true,
                col_abs: true,
            };
            let end = crate::addr::CellRef {
                sheet: Some(t.sheet),
                row: t.end_row,
                col: t.end_col,
                row_abs: true,
                col_abs: true,
            };
            out.push(Precedent::Range {
                sheet: t.sheet,
                range: RangeRef::from_corners(start, end),
                whole_col: false,
                whole_row: false,
            });
            push_cse_anchors(wb, t.sheet, RangeRef::from_corners(start, end), &mut out);
        }
    }
    out
}

fn push_cse_anchors(wb: &Workbook, sheet: SheetId, referenced: RangeRef, out: &mut Vec<Precedent>) {
    let Some(sheet_ref) = wb.sheet(sheet) else {
        return;
    };
    for formula in sheet_ref
        .array_formulas()
        .filter(|formula| ranges_intersect(formula.range, referenced))
    {
        let anchor = CellCoord::new(sheet, formula.anchor.row, formula.anchor.col);
        if !out
            .iter()
            .any(|precedent| matches!(precedent, Precedent::Cell(cell) if *cell == anchor))
        {
            out.push(Precedent::Cell(anchor));
        }
    }
}

fn ranges_intersect(a: RangeRef, b: RangeRef) -> bool {
    let a_r0 = a.start.row.min(a.end.row);
    let a_r1 = a.start.row.max(a.end.row);
    let a_c0 = a.start.col.min(a.end.col);
    let a_c1 = a.start.col.max(a.end.col);
    let b_r0 = b.start.row.min(b.end.row);
    let b_r1 = b.start.row.max(b.end.row);
    let b_c0 = b.start.col.min(b.end.col);
    let b_c1 = b.start.col.max(b.end.col);
    a_r0 <= b_r1 && b_r0 <= a_r1 && a_c0 <= b_c1 && b_c0 <= a_c1
}

enum SheetResolve {
    One(SheetId),
    Span(Vec<SheetId>),
    Missing,
}

fn resolve_sheet_spec(wb: &Workbook, spec: Option<&SheetSpec>, default: SheetId) -> SheetResolve {
    match spec {
        None => SheetResolve::One(default),
        Some(s) if s.end.is_none() => match wb.resolve_sheet_name(&s.start) {
            Ok(id) => SheetResolve::One(id),
            Err(_) => SheetResolve::Missing,
        },
        Some(s) => {
            let Ok(a) = wb.resolve_sheet_name(&s.start) else {
                return SheetResolve::Missing;
            };
            let Some(end_name) = &s.end else {
                return SheetResolve::One(a);
            };
            let Ok(b) = wb.resolve_sheet_name(end_name) else {
                return SheetResolve::Missing;
            };
            let ids: Vec<SheetId> = wb.sheets().map(|sh| sh.id).collect();
            let i = ids.iter().position(|&x| x == a);
            let j = ids.iter().position(|&x| x == b);
            match (i, j) {
                (Some(i), Some(j)) if i <= j => SheetResolve::Span(ids[i..=j].to_vec()),
                (Some(i), Some(j)) => SheetResolve::Span(ids[j..=i].to_vec()),
                _ => SheetResolve::Missing,
            }
        }
    }
}
