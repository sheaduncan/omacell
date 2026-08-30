//! Row/column geometry with Fenwick prefix sums (spec §11.3).
//!
//! Pixel ↔ index mapping is O(log n) prefix plus a binary search
//! (`O(log² n)` at 1M rows, still a few hundred operations). Hidden rows and
//! columns contribute 0 px. Empty sheets do not allocate a 1M-slot tree.

use std::collections::BTreeMap;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::error::CoreError;
use crate::limits::{MAX_COLS, MAX_ROWS};

/// Default row height in pixels (15 pt at 96 dpi).
pub const DEFAULT_ROW_PX: u32 = 20;
/// Default column width in pixels (≈ 8.43 Excel characters).
pub const DEFAULT_COL_PX: u32 = 64;

/// Sparse Fenwick tree of i64 deltas. Index 1 is the first row/col (0-based 0).
#[derive(Clone, Debug, Default)]
struct Fenwick {
    n: u32,
    tree: BTreeMap<u32, i64>,
}

impl Fenwick {
    fn new(n: u32) -> Self {
        Self {
            n,
            tree: BTreeMap::new(),
        }
    }

    fn add(&mut self, mut i: u32, delta: i64) {
        if delta == 0 || i == 0 {
            return;
        }
        while i <= self.n {
            let e = self.tree.entry(i).or_insert(0);
            *e += delta;
            if *e == 0 {
                self.tree.remove(&i);
            }
            let lsb = i & i.wrapping_neg();
            i = match i.checked_add(lsb) {
                Some(next) => next,
                None => break,
            };
        }
    }

    fn prefix(&self, mut i: u32) -> i64 {
        i = i.min(self.n);
        let mut s = 0i64;
        while i > 0 {
            if let Some(v) = self.tree.get(&i) {
                s += *v;
            }
            i -= i & i.wrapping_neg();
        }
        s
    }
}

/// One axis (rows or columns).
///
/// ```
/// use omacell_core::geometry::{AxisGeometry, DEFAULT_ROW_PX};
/// let mut a = AxisGeometry::rows();
/// a.set_size(0, 40).unwrap();
/// assert_eq!(a.index_to_pixel(1), 40);
/// assert_eq!(a.size(1).unwrap(), DEFAULT_ROW_PX);
/// ```
#[derive(Clone, Debug)]
pub struct AxisGeometry {
    max: u32,
    default_px: u32,
    fenwick: Fenwick,
    custom: FxHashMap<u32, u32>,
    hidden: FxHashSet<u32>,
    outline: FxHashMap<u32, u8>,
    collapsed: FxHashSet<u32>,
}

impl AxisGeometry {
    /// Row axis covering `MAX_ROWS`.
    #[must_use]
    pub fn rows() -> Self {
        Self::new(MAX_ROWS, DEFAULT_ROW_PX)
    }

    /// Column axis covering `MAX_COLS`.
    #[must_use]
    pub fn cols() -> Self {
        Self::new(u32::from(MAX_COLS), DEFAULT_COL_PX)
    }

    fn new(max: u32, default_px: u32) -> Self {
        Self {
            max,
            default_px,
            fenwick: Fenwick::new(max),
            custom: FxHashMap::default(),
            hidden: FxHashSet::default(),
            outline: FxHashMap::default(),
            collapsed: FxHashSet::default(),
        }
    }

    /// Count of indices on this axis.
    #[must_use]
    pub fn len(&self) -> u32 {
        self.max
    }

    /// Never empty for Excel axes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.max == 0
    }

    /// Default size in pixels.
    #[must_use]
    pub fn default_px(&self) -> u32 {
        self.default_px
    }

    fn check(&self, index: u32) -> Result<(), CoreError> {
        if index >= self.max {
            Err(CoreError::addr_ref(format!(
                "geometry index {index} is out of range (max {})",
                self.max
            )))
        } else {
            Ok(())
        }
    }

    /// Current pixel size (0 if hidden).
    pub fn size(&self, index: u32) -> Result<u32, CoreError> {
        self.check(index)?;
        Ok(self.size_unchecked(index))
    }

    fn size_unchecked(&self, index: u32) -> u32 {
        if self.hidden.contains(&index) {
            0
        } else {
            self.custom.get(&index).copied().unwrap_or(self.default_px)
        }
    }

    /// Whether the index is hidden.
    pub fn is_hidden(&self, index: u32) -> Result<bool, CoreError> {
        self.check(index)?;
        Ok(self.hidden.contains(&index))
    }

    /// Custom sizes `(index, px)` in ascending index order (WP-10 writer).
    pub fn iter_custom(&self) -> impl Iterator<Item = (u32, u32)> {
        let mut v: Vec<(u32, u32)> = self.custom.iter().map(|(&i, &px)| (i, px)).collect();
        v.sort_unstable_by_key(|(i, _)| *i);
        v.into_iter()
    }

    /// Hidden indices in ascending order (WP-10 writer).
    pub fn iter_hidden(&self) -> impl Iterator<Item = u32> {
        let mut v: Vec<u32> = self.hidden.iter().copied().collect();
        v.sort_unstable();
        v.into_iter()
    }

    /// Outline level (0–7). Excel default is 0.
    #[must_use]
    pub fn outline_level(&self, index: u32) -> u8 {
        self.outline.get(&index).copied().unwrap_or(0)
    }

    /// Set outline level. 0 clears.
    pub fn set_outline_level(&mut self, index: u32, level: u8) -> Result<(), CoreError> {
        self.check(index)?;
        if level == 0 {
            self.outline.remove(&index);
        } else {
            self.outline.insert(index, level.min(7));
        }
        Ok(())
    }

    /// Whether the outline group at `index` is collapsed.
    #[must_use]
    pub fn is_collapsed(&self, index: u32) -> bool {
        self.collapsed.contains(&index)
    }

    /// Collapse or expand an outline group.
    pub fn set_collapsed(&mut self, index: u32, collapsed: bool) -> Result<(), CoreError> {
        self.check(index)?;
        if collapsed {
            self.collapsed.insert(index);
        } else {
            self.collapsed.remove(&index);
        }
        Ok(())
    }

    /// `(index, level)` in ascending order for the writer.
    pub fn iter_outline(&self) -> impl Iterator<Item = (u32, u8)> {
        let mut v: Vec<(u32, u8)> = self.outline.iter().map(|(&i, &l)| (i, l)).collect();
        v.sort_unstable_by_key(|(i, _)| *i);
        v.into_iter()
    }

    /// Shift hidden/custom/outline maps when inserting or deleting `count` items at `at`.
    pub fn shift_meta(&mut self, at: u32, count: i32) -> Result<(), CoreError> {
        if count == 0 {
            return Ok(());
        }
        let mag = count.unsigned_abs();
        if count > 0 {
            if at.saturating_add(mag) > self.max {
                return Err(CoreError::addr_ref("geometry insert exceeds the axis"));
            }
            shift_map_insert(&mut self.custom, at, mag, self.max);
            shift_set_insert(&mut self.hidden, at, mag, self.max);
            shift_map_insert(&mut self.outline, at, mag, self.max);
            shift_set_insert(&mut self.collapsed, at, mag, self.max);
            // Fenwick: rebuild from custom+hidden
            self.rebuild_fenwick();
        } else {
            shift_map_delete(&mut self.custom, at, mag);
            shift_set_delete(&mut self.hidden, at, mag);
            shift_map_delete(&mut self.outline, at, mag);
            shift_set_delete(&mut self.collapsed, at, mag);
            self.rebuild_fenwick();
        }
        Ok(())
    }

    fn rebuild_fenwick(&mut self) {
        self.fenwick = Fenwick::new(self.max);
        let mut idxs: FxHashSet<u32> = self.custom.keys().copied().collect();
        idxs.extend(self.hidden.iter().copied());
        for i in idxs {
            let sz = self.size_unchecked(i);
            let delta = i64::from(sz) - i64::from(self.default_px);
            if delta != 0 {
                self.fenwick.add(i + 1, delta);
            }
        }
    }

    /// Set a custom size in pixels. Hidden rows keep a stored size for unhide.
    pub fn set_size(&mut self, index: u32, px: u32) -> Result<(), CoreError> {
        self.check(index)?;
        let old = self.size_unchecked(index);
        self.custom.insert(index, px);
        if !self.hidden.contains(&index) {
            self.fenwick.add(index + 1, i64::from(px) - i64::from(old));
        }
        Ok(())
    }

    /// Hide or unhide. Hidden indices contribute 0 px.
    pub fn set_hidden(&mut self, index: u32, hidden: bool) -> Result<(), CoreError> {
        self.check(index)?;
        let was = self.hidden.contains(&index);
        if was == hidden {
            return Ok(());
        }
        if hidden {
            let old = self.size_unchecked(index);
            self.hidden.insert(index);
            self.fenwick.add(index + 1, -i64::from(old));
        } else {
            self.hidden.remove(&index);
            let now = self.size_unchecked(index);
            self.fenwick.add(index + 1, i64::from(now));
        }
        Ok(())
    }

    /// Batch size updates.
    pub fn set_sizes(
        &mut self,
        items: impl IntoIterator<Item = (u32, u32)>,
    ) -> Result<(), CoreError> {
        for (i, px) in items {
            self.set_size(i, px)?;
        }
        Ok(())
    }

    /// Pixel coordinate of the top/left edge of `index`.
    ///
    /// `index_to_pixel(0) == 0`. `index_to_pixel(max)` is the total span.
    #[must_use]
    pub fn index_to_pixel(&self, index: u32) -> u64 {
        let i = index.min(self.max);
        let v = i128::from(i) * i128::from(self.default_px) + i128::from(self.fenwick.prefix(i));
        if v < 0 { 0 } else { v as u64 }
    }

    /// Total pixel span of the axis.
    #[must_use]
    pub fn total_px(&self) -> u64 {
        self.index_to_pixel(self.max)
    }

    /// Index whose pixel span contains `px`. Hidden (0-px) indices are skipped.
    #[must_use]
    pub fn pixel_to_index(&self, px: u64) -> u32 {
        if self.max == 0 {
            return 0;
        }
        let total = self.total_px();
        if total == 0 {
            return 0;
        }
        let px = px.min(total - 1);
        let mut lo = 0u32;
        let mut hi = self.max;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.index_to_pixel(mid) <= px {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let mut i = lo.saturating_sub(1);
        while i + 1 < self.max && self.size_unchecked(i) == 0 {
            i += 1;
        }
        i
    }
}

fn shift_map_insert<T: Copy>(map: &mut FxHashMap<u32, T>, at: u32, mag: u32, max: u32) {
    let mut next = FxHashMap::default();
    for (&i, &v) in map.iter() {
        let ni = if i >= at { i.saturating_add(mag) } else { i };
        if ni < max {
            next.insert(ni, v);
        }
    }
    *map = next;
}

fn shift_map_delete<T: Copy>(map: &mut FxHashMap<u32, T>, at: u32, mag: u32) {
    let end = at.saturating_add(mag);
    let mut next = FxHashMap::default();
    for (&i, &v) in map.iter() {
        if i < at {
            next.insert(i, v);
        } else if i >= end {
            next.insert(i - mag, v);
        }
    }
    *map = next;
}

fn shift_set_insert(set: &mut FxHashSet<u32>, at: u32, mag: u32, max: u32) {
    let mut next = FxHashSet::default();
    for &i in set.iter() {
        let ni = if i >= at { i.saturating_add(mag) } else { i };
        if ni < max {
            next.insert(ni);
        }
    }
    *set = next;
}

fn shift_set_delete(set: &mut FxHashSet<u32>, at: u32, mag: u32) {
    let end = at.saturating_add(mag);
    let mut next = FxHashSet::default();
    for &i in set.iter() {
        if i < at {
            next.insert(i);
        } else if i >= end {
            next.insert(i - mag);
        }
    }
    *set = next;
}

/// Row and column geometry for one sheet.
///
/// ```
/// use omacell_core::geometry::SheetGeometry;
/// let g = SheetGeometry::new();
/// assert_eq!(g.rows.index_to_pixel(1), 20);
/// assert_eq!(g.cols.index_to_pixel(1), 64);
/// ```
#[derive(Clone, Debug)]
pub struct SheetGeometry {
    /// Row heights.
    pub rows: AxisGeometry,
    /// Column widths.
    pub cols: AxisGeometry,
}

impl SheetGeometry {
    /// Default Excel-like sizes, nothing hidden.
    #[must_use]
    pub fn new() -> Self {
        Self {
            rows: AxisGeometry::rows(),
            cols: AxisGeometry::cols(),
        }
    }
}

impl Default for SheetGeometry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mapping() {
        let a = AxisGeometry::rows();
        assert_eq!(a.index_to_pixel(0), 0);
        assert_eq!(a.index_to_pixel(10), 10 * u64::from(DEFAULT_ROW_PX));
        assert_eq!(a.pixel_to_index(0), 0);
        assert_eq!(a.pixel_to_index(DEFAULT_ROW_PX as u64), 1);
        assert_eq!(a.pixel_to_index(DEFAULT_ROW_PX as u64 - 1), 0);
    }

    #[test]
    fn custom_and_hidden() {
        let mut a = AxisGeometry::rows();
        a.set_size(0, 40).unwrap();
        a.set_size(2, 10).unwrap();
        a.set_hidden(1, true).unwrap();
        assert_eq!(a.size(0).unwrap(), 40);
        assert_eq!(a.size(1).unwrap(), 0);
        assert_eq!(a.size(2).unwrap(), 10);
        assert_eq!(a.index_to_pixel(1), 40);
        assert_eq!(a.index_to_pixel(2), 40);
        assert_eq!(a.index_to_pixel(3), 50);
        assert_eq!(a.pixel_to_index(0), 0);
        assert_eq!(a.pixel_to_index(39), 0);
        assert_eq!(a.pixel_to_index(40), 2);
        assert_eq!(a.pixel_to_index(49), 2);
        a.set_hidden(1, false).unwrap();
        assert_eq!(a.size(1).unwrap(), DEFAULT_ROW_PX);
        assert_eq!(a.index_to_pixel(2), 40 + u64::from(DEFAULT_ROW_PX));
    }

    #[test]
    fn pixel_index_roundtrip_visible() {
        let mut a = AxisGeometry::rows();
        for i in [0u32, 5, 100, 10_000] {
            a.set_size(i, 12 + (i % 7)).unwrap();
        }
        a.set_hidden(5, true).unwrap();
        for i in 0..200u32 {
            if a.size(i).unwrap() == 0 {
                continue;
            }
            let y = a.index_to_pixel(i);
            assert_eq!(a.pixel_to_index(y), i, "i={i} y={y}");
        }
    }

    #[test]
    fn out_of_range() {
        let mut a = AxisGeometry::cols();
        assert!(a.set_size(u32::from(MAX_COLS), 10).is_err());
    }
}
