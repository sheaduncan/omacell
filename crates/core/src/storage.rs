//! Block-sparse cell storage (spec §11.3).
//!
//! Cells live in 256×256 blocks keyed by block coordinate. Each block has an
//! occupancy bitmap and a packed dense [`CellSlot`] array in row-major set-bit
//! order. Blocks are `Arc` so a workbook snapshot is copy-on-write.

use std::sync::Arc;

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::intern::FormulaId;
use crate::limits::{MAX_COLS, MAX_ROWS};
use crate::style::StyleId;
use crate::value::Value;

/// Block edge in cells (spec §11.3).
pub const BLOCK_SIZE: u32 = 256;

const BLOCK: usize = 256;
const WORDS: usize = (BLOCK * BLOCK) / 64;

/// Packed per-cell flags (locked / hidden-formula / dirty / spill / CSE / stale).
///
/// Excel’s default is locked. Dirty, spill, CSE, and stale are used by WP-04.
///
/// ```
/// use omacell_core::storage::CellFlags;
/// assert!(CellFlags::DEFAULT.locked());
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CellFlags(u8);

impl CellFlags {
    /// Locked, not hidden, not dirty.
    pub const DEFAULT: Self = Self(Self::LOCKED);
    /// Sheet-protection lock.
    pub const LOCKED: u8 = 1 << 0;
    /// Hide formula when the sheet is protected.
    pub const HIDDEN: u8 = 1 << 1;
    /// Needs recalculation (WP-04).
    pub const DIRTY: u8 = 1 << 2;
    /// Spill-ghost cell written by a dynamic array (WP-04).
    pub const SPILL: u8 = 1 << 3;
    /// Legacy CSE array formula: evaluate without spilling (WP-04).
    pub const ARRAY: u8 = 1 << 4;
    /// Async/AI result is stale (WP-04, A-3.3).
    pub const STALE: u8 = 1 << 5;

    /// Empty flags (unlocked).
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Packed bits.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Locked flag.
    #[must_use]
    pub const fn locked(self) -> bool {
        self.0 & Self::LOCKED != 0
    }

    /// Hidden-formula flag.
    #[must_use]
    pub const fn hidden(self) -> bool {
        self.0 & Self::HIDDEN != 0
    }

    /// Dirty flag.
    #[must_use]
    pub const fn dirty(self) -> bool {
        self.0 & Self::DIRTY != 0
    }

    /// Spill-ghost flag.
    #[must_use]
    pub const fn spill(self) -> bool {
        self.0 & Self::SPILL != 0
    }

    /// Legacy CSE array-formula flag.
    #[must_use]
    pub const fn array(self) -> bool {
        self.0 & Self::ARRAY != 0
    }

    /// Async/AI stale flag.
    #[must_use]
    pub const fn stale(self) -> bool {
        self.0 & Self::STALE != 0
    }

    /// Set or clear a bit.
    #[must_use]
    pub const fn with(self, bit: u8, on: bool) -> Self {
        if on {
            Self(self.0 | bit)
        } else {
            Self(self.0 & !bit)
        }
    }
}

/// Compact cell payload stored in a block slot (spec §11.3).
///
/// Notes, comments, and hyperlinks live in side tables on [`crate::sheet::Sheet`].
///
/// ```
/// use omacell_core::storage::CellSlot;
/// let s = CellSlot::number(1.5);
/// assert!(matches!(s.value, omacell_core::value::Value::Number(_)));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellSlot {
    /// Cached / literal value.
    pub value: Value,
    /// Interned formula source. `None` means a literal or empty cell.
    /// WP-03 parses this into an AST; WP-02 stores the handle only.
    pub formula: Option<FormulaId>,
    /// Interned style.
    pub style: StyleId,
    /// Packed flags.
    pub flags: CellFlags,
}

const _: () = assert!(
    std::mem::size_of::<CellSlot>() <= 32,
    "CellSlot must stay compact so 1M×20 numeric cells fit the 64 B/cell budget"
);

impl CellSlot {
    /// A numeric cell with the default style and flags.
    #[must_use]
    pub fn number(n: f64) -> Self {
        Self {
            value: Value::Number(n),
            formula: None,
            style: StyleId::DEFAULT,
            flags: CellFlags::DEFAULT,
        }
    }

    /// An empty cell with default style (still occupies a slot if stored).
    #[must_use]
    pub fn empty() -> Self {
        Self {
            value: Value::Empty,
            formula: None,
            style: StyleId::DEFAULT,
            flags: CellFlags::DEFAULT,
        }
    }

    /// Whether this slot is a plain numeric with no formula and default style.
    #[must_use]
    pub fn is_plain_numeric(self) -> bool {
        matches!(self.value, Value::Number(_))
            && self.formula.is_none()
            && self.style == StyleId::DEFAULT
    }
}

/// Inclusive bounding box of occupied cells.
///
/// ```
/// use omacell_core::storage::UsedRange;
/// let u = UsedRange { min_row: 0, min_col: 0, max_row: 1, max_col: 1 };
/// assert_eq!(u.cells(), 4);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UsedRange {
    /// Minimum occupied row.
    pub min_row: u32,
    /// Minimum occupied column.
    pub min_col: u16,
    /// Maximum occupied row (inclusive).
    pub max_row: u32,
    /// Maximum occupied column (inclusive).
    pub max_col: u16,
}

impl UsedRange {
    fn single(row: u32, col: u16) -> Self {
        Self {
            min_row: row,
            min_col: col,
            max_row: row,
            max_col: col,
        }
    }

    fn include(&mut self, row: u32, col: u16) {
        self.min_row = self.min_row.min(row);
        self.max_row = self.max_row.max(row);
        self.min_col = self.min_col.min(col);
        self.max_col = self.max_col.max(col);
    }

    /// Inclusive cell count of the rectangle (including empty holes).
    #[must_use]
    pub fn cells(self) -> u64 {
        let rows = u64::from(self.max_row - self.min_row) + 1;
        let cols = u64::from(self.max_col - self.min_col) + 1;
        rows * cols
    }
}

/// Coordinate of a 256×256 block.
///
/// ```
/// use omacell_core::storage::{BlockCoord, BLOCK_SIZE};
/// let c = BlockCoord::from_cell(256, 0);
/// assert_eq!(c.brow, 1);
/// assert_eq!(BLOCK_SIZE, 256);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockCoord {
    /// `row / 256`.
    pub brow: u16,
    /// `col / 256`.
    pub bcol: u8,
}

impl BlockCoord {
    /// Block containing `(row, col)`.
    #[must_use]
    pub fn from_cell(row: u32, col: u16) -> Self {
        Self {
            brow: (row / BLOCK_SIZE) as u16,
            bcol: (col / BLOCK_SIZE as u16) as u8,
        }
    }

    fn origin(self) -> (u32, u16) {
        (
            u32::from(self.brow) * BLOCK_SIZE,
            u16::from(self.bcol) * BLOCK_SIZE as u16,
        )
    }
}

#[derive(Clone, Debug)]
struct Block {
    bits: Box<[u64]>,
    slots: Vec<CellSlot>,
    last_bit: Option<u16>,
}

impl Block {
    fn empty() -> Self {
        Self {
            bits: vec![0u64; WORDS].into_boxed_slice(),
            slots: Vec::new(),
            last_bit: None,
        }
    }

    fn bit(r: u8, c: u8) -> usize {
        (r as usize) * BLOCK + c as usize
    }

    fn rank(&self, bit: usize) -> usize {
        let w = bit / 64;
        let r = bit % 64;
        let mut n = 0usize;
        for word in self.bits.iter().take(w) {
            n += word.count_ones() as usize;
        }
        if r > 0 {
            n += (self.bits[w] & ((1u64 << r) - 1)).count_ones() as usize;
        }
        n
    }

    fn occupied(&self, bit: usize) -> bool {
        let w = bit / 64;
        let r = bit % 64;
        self.bits[w] & (1u64 << r) != 0
    }

    fn get(&self, r: u8, c: u8) -> Option<&CellSlot> {
        let bit = Self::bit(r, c);
        if !self.occupied(bit) {
            return None;
        }
        Some(&self.slots[self.rank(bit)])
    }

    fn set(&mut self, r: u8, c: u8, slot: CellSlot) -> Option<CellSlot> {
        let bit = Self::bit(r, c);
        if !self.occupied(bit) && self.last_bit.is_none_or(|last| bit > usize::from(last)) {
            let w = bit / 64;
            let b = bit % 64;
            self.bits[w] |= 1u64 << b;
            self.slots.push(slot);
            self.last_bit = Some(bit as u16);
            return None;
        }
        let i = self.rank(bit);
        if self.occupied(bit) {
            let old = self.slots[i];
            self.slots[i] = slot;
            Some(old)
        } else {
            let w = bit / 64;
            let b = bit % 64;
            self.bits[w] |= 1u64 << b;
            self.slots.insert(i, slot);
            self.last_bit = Some(
                self.last_bit
                    .map_or(bit as u16, |last| last.max(bit as u16)),
            );
            None
        }
    }

    fn clear(&mut self, r: u8, c: u8) -> Option<CellSlot> {
        let bit = Self::bit(r, c);
        if !self.occupied(bit) {
            return None;
        }
        let i = self.rank(bit);
        let w = bit / 64;
        let b = bit % 64;
        self.bits[w] &= !(1u64 << b);
        let old = self.slots.remove(i);
        if self.last_bit == Some(bit as u16) {
            self.last_bit = self
                .bits
                .iter()
                .enumerate()
                .rev()
                .find_map(|(word_idx, word)| {
                    (*word != 0).then(|| {
                        let high = 63 - word.leading_zeros() as usize;
                        (word_idx * 64 + high) as u16
                    })
                });
        }
        Some(old)
    }

    fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    fn iter_cells(&self, coord: BlockCoord) -> impl Iterator<Item = (u32, u16, CellSlot)> + '_ {
        let (orow, ocol) = coord.origin();
        BlockIter {
            bits: &self.bits,
            slots: &self.slots,
            word: 0,
            bit: 0,
            slot_i: 0,
            orow,
            ocol,
        }
    }

    fn heap_bytes(&self) -> usize {
        size_of::<Self>() + self.bits.len() * 8 + self.slots.capacity() * size_of::<CellSlot>()
    }
}

struct BlockIter<'a> {
    bits: &'a [u64],
    slots: &'a [CellSlot],
    word: usize,
    bit: u32,
    slot_i: usize,
    orow: u32,
    ocol: u16,
}

impl Iterator for BlockIter<'_> {
    type Item = (u32, u16, CellSlot);

    fn next(&mut self) -> Option<Self::Item> {
        while self.word < self.bits.len() {
            let w = self.bits[self.word] >> self.bit;
            if w == 0 {
                self.word += 1;
                self.bit = 0;
                continue;
            }
            let tz = w.trailing_zeros();
            let bit = self.bit + tz;
            let abs_bit = self.word * 64 + bit as usize;
            self.bit = bit + 1;
            if self.bit == 64 {
                self.word += 1;
                self.bit = 0;
            }
            let r = (abs_bit / BLOCK) as u32;
            let c = (abs_bit % BLOCK) as u16;
            let slot = self.slots[self.slot_i];
            self.slot_i += 1;
            return Some((self.orow + r, self.ocol + c, slot));
        }
        None
    }
}

/// Per-sheet sparse grid.
///
/// ```
/// use omacell_core::storage::{CellSlot, SheetStore};
/// let mut s = SheetStore::new();
/// s.set(0, 0, CellSlot::number(1.0)).unwrap();
/// assert_eq!(s.get(0, 0).unwrap().unwrap().value, omacell_core::value::Value::Number(1.0));
/// ```
#[derive(Clone, Debug, Default)]
pub struct SheetStore {
    blocks: FxHashMap<BlockCoord, Arc<Block>>,
    used: Option<UsedRange>,
    live: u64,
}

impl SheetStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Occupied cell count.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.live
    }

    /// Whether no cells are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    fn check(row: u32, col: u16) -> Result<(), CoreError> {
        if row >= MAX_ROWS || u32::from(col) >= u32::from(MAX_COLS) {
            Err(CoreError::addr_ref(format!(
                "cell r{row}c{col} is out of range"
            )))
        } else {
            Ok(())
        }
    }

    /// Borrow a slot, or `None` if the cell is not stored.
    pub fn get(&self, row: u32, col: u16) -> Result<Option<&CellSlot>, CoreError> {
        Self::check(row, col)?;
        let coord = BlockCoord::from_cell(row, col);
        let Some(block) = self.blocks.get(&coord) else {
            return Ok(None);
        };
        let (orow, ocol) = coord.origin();
        Ok(block.get((row - orow) as u8, (col - ocol) as u8))
    }

    /// Insert or replace a slot. Returns the previous occupant.
    pub fn set(
        &mut self,
        row: u32,
        col: u16,
        slot: CellSlot,
    ) -> Result<Option<CellSlot>, CoreError> {
        Self::check(row, col)?;
        let coord = BlockCoord::from_cell(row, col);
        let (orow, ocol) = coord.origin();
        let block = Arc::make_mut(
            self.blocks
                .entry(coord)
                .or_insert_with(|| Arc::new(Block::empty())),
        );
        let old = block.set((row - orow) as u8, (col - ocol) as u8, slot);
        if old.is_none() {
            self.live += 1;
            match &mut self.used {
                Some(u) => u.include(row, col),
                None => self.used = Some(UsedRange::single(row, col)),
            }
        }
        Ok(old)
    }

    /// Remove a slot.
    pub fn clear(&mut self, row: u32, col: u16) -> Result<Option<CellSlot>, CoreError> {
        Self::check(row, col)?;
        let coord = BlockCoord::from_cell(row, col);
        let Some(block_arc) = self.blocks.get_mut(&coord) else {
            return Ok(None);
        };
        let (orow, ocol) = coord.origin();
        let block = Arc::make_mut(block_arc);
        let old = block.clear((row - orow) as u8, (col - ocol) as u8);
        let drop_block = block.is_empty();
        if drop_block {
            self.blocks.remove(&coord);
        }
        if old.is_some() {
            self.live -= 1;
            if let Some(u) = self.used
                && (row == u.min_row || row == u.max_row || col == u.min_col || col == u.max_col)
            {
                self.recompute_used();
            }
        }
        Ok(old)
    }

    fn recompute_used(&mut self) {
        let mut used = None;
        for (r, c, _) in self.iter() {
            match &mut used {
                Some(u) => UsedRange::include(u, r, c),
                None => used = Some(UsedRange::single(r, c)),
            }
        }
        self.used = used;
    }

    /// Tight bounding box of occupied cells.
    #[must_use]
    pub fn used_range(&self) -> Option<UsedRange> {
        self.used
    }

    /// Bottom-right of the used range (Excel `dimension` end). `None` if empty.
    #[must_use]
    pub fn dimension(&self) -> Option<(u32, u16)> {
        self.used.map(|u| (u.max_row, u.max_col))
    }

    /// Row-major iteration of occupied cells, in sorted block order.
    pub fn iter(&self) -> impl Iterator<Item = (u32, u16, CellSlot)> + '_ {
        let mut coords: Vec<BlockCoord> = self.blocks.keys().copied().collect();
        coords.sort_unstable();
        coords.into_iter().flat_map(|coord| {
            self.blocks
                .get(&coord)
                .map(|b| b.iter_cells(coord))
                .into_iter()
                .flatten()
        })
    }

    /// Occupied cells inside an inclusive rectangle, row-major.
    pub fn iter_region(
        &self,
        row0: u32,
        col0: u16,
        row1: u32,
        col1: u16,
    ) -> impl Iterator<Item = (u32, u16, CellSlot)> + '_ {
        let (r0, r1) = if row0 <= row1 {
            (row0, row1)
        } else {
            (row1, row0)
        };
        let (c0, c1) = if col0 <= col1 {
            (col0, col1)
        } else {
            (col1, col0)
        };
        self.iter()
            .filter(move |(r, c, _)| *r >= r0 && *r <= r1 && *c >= c0 && *c <= c1)
    }

    /// Insert (`count > 0`) or delete (`count < 0`) rows at `at`.
    ///
    /// Occupied cells that would land outside the grid make this a no-op error
    /// (`addr.ref`). Formula *token* rewrite is WP-03 / WP-17; this only moves
    /// slots.
    pub fn shift_rows(&mut self, at: u32, count: i32) -> Result<(), CoreError> {
        if count == 0 {
            return Ok(());
        }
        if at >= MAX_ROWS {
            return Err(CoreError::addr_ref(format!(
                "row shift anchor {at} is out of range"
            )));
        }
        let cells: Vec<(u32, u16, CellSlot)> = self.iter().collect();
        let magnitude = count.unsigned_abs();
        if count > 0 {
            let n = magnitude;
            if n > MAX_ROWS - at {
                return Err(CoreError::addr_ref(format!(
                    "inserting {n} rows at {at} exceeds the worksheet grid"
                )));
            }
            for (r, _, _) in &cells {
                if *r >= at {
                    let nr = r
                        .checked_add(n)
                        .ok_or_else(|| CoreError::addr_ref("row insert overflows u32"))?;
                    if nr >= MAX_ROWS {
                        return Err(CoreError::addr_ref(format!(
                            "inserting {n} rows at {at} would push a cell past row {MAX_ROWS}"
                        )));
                    }
                }
            }
        }
        let mut next = SheetStore::new();
        let delete_end = at.saturating_add(magnitude).min(MAX_ROWS);
        let deleted = delete_end - at;
        for (r, c, slot) in cells {
            if count > 0 {
                let nr = if r >= at { r + magnitude } else { r };
                next.set(nr, c, slot)?;
            } else if r < at {
                next.set(r, c, slot)?;
            } else if r >= delete_end {
                next.set(r - deleted, c, slot)?;
            }
        }
        *self = next;
        Ok(())
    }

    /// Insert or delete columns at `at`. Same overflow rules as [`Self::shift_rows`].
    pub fn shift_cols(&mut self, at: u16, count: i32) -> Result<(), CoreError> {
        if count == 0 {
            return Ok(());
        }
        if u32::from(at) >= u32::from(MAX_COLS) {
            return Err(CoreError::addr_ref(format!(
                "column shift anchor {at} is out of range"
            )));
        }
        let cells: Vec<(u32, u16, CellSlot)> = self.iter().collect();
        let magnitude = count.unsigned_abs();
        let at_u32 = u32::from(at);
        if count > 0 {
            let n = magnitude;
            if n > u32::from(MAX_COLS) - at_u32 {
                return Err(CoreError::addr_ref(format!(
                    "inserting {n} columns at {at} exceeds the worksheet grid"
                )));
            }
            for (_, c, _) in &cells {
                if *c >= at {
                    let nc = u32::from(*c) + n;
                    if nc >= u32::from(MAX_COLS) {
                        return Err(CoreError::addr_ref(format!(
                            "inserting {n} columns at {at} would push a cell past column {MAX_COLS}"
                        )));
                    }
                }
            }
        }
        let mut next = SheetStore::new();
        let delete_end = at_u32.saturating_add(magnitude).min(u32::from(MAX_COLS));
        let deleted = delete_end - at_u32;
        for (r, c, slot) in cells {
            if count > 0 {
                let nc = if c >= at {
                    u16::try_from(u32::from(c) + magnitude)
                        .map_err(|_| CoreError::addr_ref("column insert overflows u16"))?
                } else {
                    c
                };
                next.set(r, nc, slot)?;
            } else if c < at {
                next.set(r, c, slot)?;
            } else if u32::from(c) >= delete_end {
                let nc = u16::try_from(u32::from(c) - deleted)
                    .map_err(|_| CoreError::addr_ref("column delete underflows u16"))?;
                next.set(r, nc, slot)?;
            }
        }
        *self = next;
        Ok(())
    }

    /// Drop spare slot capacity in every block (layout-budget measurement).
    pub fn shrink_to_fit(&mut self) {
        for block in self.blocks.values_mut() {
            Arc::make_mut(block).slots.shrink_to_fit();
        }
    }

    /// Estimated heap bytes of blocks + slot storage (not intern tables).
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        let mut n = crate::intern::hashmap_bytes(
            self.blocks.capacity(),
            size_of::<(BlockCoord, Arc<Block>)>(),
        );
        for b in self.blocks.values() {
            n += 16; // Arc inner header
            n += b.heap_bytes();
        }
        n
    }
}

use std::mem::size_of;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_slot_is_at_most_32_bytes() {
        assert!(
            size_of::<CellSlot>() <= 32,
            "CellSlot is {} bytes",
            size_of::<CellSlot>()
        );
    }

    #[test]
    fn set_get_clear_one_cell() {
        let mut s = SheetStore::new();
        assert!(s.get(0, 0).unwrap().is_none());
        assert!(s.set(0, 0, CellSlot::number(2.0)).unwrap().is_none());
        assert_eq!(s.get(0, 0).unwrap().unwrap().value, Value::Number(2.0));
        let old = s.set(0, 0, CellSlot::number(3.0)).unwrap();
        assert_eq!(old.unwrap().value, Value::Number(2.0));
        assert_eq!(s.clear(0, 0).unwrap().unwrap().value, Value::Number(3.0));
        assert!(s.get(0, 0).unwrap().is_none());
        assert!(s.is_empty());
    }

    #[test]
    fn out_of_range_is_ref() {
        let mut s = SheetStore::new();
        assert_eq!(
            s.set(MAX_ROWS, 0, CellSlot::number(1.0)).unwrap_err().code,
            crate::error::codes::ADDR_REF
        );
        assert_eq!(
            s.get(0, MAX_COLS).unwrap_err().code,
            crate::error::codes::ADDR_REF
        );
    }

    #[test]
    fn used_range_tracks_bounds() {
        let mut s = SheetStore::new();
        s.set(2, 3, CellSlot::number(1.0)).unwrap();
        s.set(10, 1, CellSlot::number(1.0)).unwrap();
        let u = s.used_range().unwrap();
        assert_eq!(
            u,
            UsedRange {
                min_row: 2,
                min_col: 1,
                max_row: 10,
                max_col: 3
            }
        );
        s.clear(10, 1).unwrap();
        let u = s.used_range().unwrap();
        assert_eq!(u.max_row, 2);
        assert_eq!(u.max_col, 3);
    }

    #[test]
    fn shift_rows_insert_and_delete() {
        let mut s = SheetStore::new();
        s.set(0, 0, CellSlot::number(1.0)).unwrap();
        s.set(2, 0, CellSlot::number(2.0)).unwrap();
        s.shift_rows(1, 2).unwrap();
        assert_eq!(s.get(0, 0).unwrap().unwrap().value, Value::Number(1.0));
        assert!(s.get(2, 0).unwrap().is_none());
        assert_eq!(s.get(4, 0).unwrap().unwrap().value, Value::Number(2.0));
        s.shift_rows(1, -2).unwrap();
        assert_eq!(s.get(2, 0).unwrap().unwrap().value, Value::Number(2.0));
    }

    #[test]
    fn shift_rows_refuses_overflow() {
        let mut s = SheetStore::new();
        s.set(MAX_ROWS - 1, 0, CellSlot::number(1.0)).unwrap();
        assert_eq!(
            s.shift_rows(0, 1).unwrap_err().code,
            crate::error::codes::ADDR_REF
        );
        assert!(s.get(MAX_ROWS - 1, 0).unwrap().is_some());
    }

    #[test]
    fn shifts_reject_bad_anchors_and_extreme_insert_counts() {
        let mut s = SheetStore::new();
        assert!(s.shift_rows(MAX_ROWS, 1).is_err());
        assert!(s.shift_cols(MAX_COLS, 1).is_err());
        assert!(s.shift_rows(0, i32::MAX).is_err());
        assert!(s.shift_cols(0, i32::MAX).is_err());
        assert!(s.shift_rows(0, i32::MIN).is_ok());
        assert!(s.shift_cols(0, i32::MIN).is_ok());
    }

    #[test]
    fn iter_is_row_major() {
        let mut s = SheetStore::new();
        s.set(1, 1, CellSlot::number(1.0)).unwrap();
        s.set(0, 2, CellSlot::number(2.0)).unwrap();
        s.set(0, 0, CellSlot::number(3.0)).unwrap();
        let pos: Vec<_> = s.iter().map(|(r, c, _)| (r, c)).collect();
        assert_eq!(pos, vec![(0, 0), (0, 2), (1, 1)]);
    }

    #[test]
    fn cow_snapshot_is_independent() {
        let mut a = SheetStore::new();
        a.set(0, 0, CellSlot::number(1.0)).unwrap();
        let snap = a.clone();
        a.set(0, 0, CellSlot::number(2.0)).unwrap();
        a.set(1, 0, CellSlot::number(3.0)).unwrap();
        assert_eq!(snap.get(0, 0).unwrap().unwrap().value, Value::Number(1.0));
        assert!(snap.get(1, 0).unwrap().is_none());
        assert_eq!(a.get(0, 0).unwrap().unwrap().value, Value::Number(2.0));
    }
}
