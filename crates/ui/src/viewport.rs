//! Virtualized viewport: frozen panes, split, zoom, hidden rows.

use omacell_core::geometry::{AxisGeometry, DEFAULT_COL_PX, DEFAULT_ROW_PX};
use omacell_core::sheet::{FreezePanes, SplitView};

/// Visible window over a sheet.
#[derive(Clone, Debug)]
pub struct Viewport {
    /// First non-frozen visible row.
    pub first_row: u32,
    /// First non-frozen visible column.
    pub first_col: u16,
    /// Viewport width in CSS pixels at zoom 1.
    pub width_px: u32,
    /// Viewport height in CSS pixels at zoom 1.
    pub height_px: u32,
    /// Zoom factor (1.0 = 100%).
    pub zoom: f64,
    /// Frozen header counts.
    pub freeze: FreezePanes,
    /// Optional split.
    pub split: Option<SplitView>,
    /// Row geometry (includes hidden).
    pub rows: AxisGeometry,
    /// Column geometry.
    pub cols: AxisGeometry,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            first_row: 0,
            first_col: 0,
            width_px: 800,
            height_px: 600,
            zoom: 1.0,
            freeze: FreezePanes::default(),
            split: None,
            rows: AxisGeometry::rows(),
            cols: AxisGeometry::cols(),
        }
    }
}

impl Viewport {
    /// Zoom in/out/reset. Chrome is unaffected; grid only.
    pub fn set_zoom(&mut self, zoom: f64) {
        self.zoom = zoom.clamp(0.25, 8.0);
    }

    /// Pixel size of row `index` at current zoom (0 if hidden).
    #[must_use]
    pub fn row_px(&self, index: u32) -> u32 {
        let base = self.rows.size(index).unwrap_or(DEFAULT_ROW_PX);
        scaled(base, self.zoom)
    }

    /// Pixel size of column `index` at current zoom (0 if hidden).
    #[must_use]
    pub fn col_px(&self, index: u16) -> u32 {
        let base = self.cols.size(u32::from(index)).unwrap_or(DEFAULT_COL_PX);
        scaled(base, self.zoom)
    }

    /// First fully visible data row after frozen rows, skipping hidden.
    #[must_use]
    pub fn first_data_row(&self) -> u32 {
        skip_hidden_row(&self.rows, self.first_row.max(self.freeze.rows))
    }

    /// Hit-test a grid-local pixel to a row index, skipping hidden rows.
    #[must_use]
    pub fn hit_row(&self, y_px: u64) -> u32 {
        let y = (y_px as f64 / self.zoom.max(0.01)) as u64;
        self.rows.pixel_to_index(y)
    }

    /// Hit-test a grid-local pixel to a column index, skipping hidden columns.
    #[must_use]
    pub fn hit_col(&self, x_px: u64) -> u16 {
        let x = (x_px as f64 / self.zoom.max(0.01)) as u64;
        self.cols.pixel_to_index(x) as u16
    }

    /// Scroll so `row` is inside the data window.
    pub fn ensure_row_visible(&mut self, row: u32) {
        if row < self.freeze.rows {
            return;
        }
        if row < self.first_row {
            self.first_row = row;
        }
    }

    /// Scroll so `col` is inside the data window.
    pub fn ensure_col_visible(&mut self, col: u16) {
        if col < self.freeze.cols {
            return;
        }
        if col < self.first_col {
            self.first_col = col;
        }
    }
}

fn scaled(px: u32, zoom: f64) -> u32 {
    ((f64::from(px) * zoom).round() as u32).max(if px == 0 { 0 } else { 1 })
}

fn skip_hidden_row(rows: &AxisGeometry, mut index: u32) -> u32 {
    while rows.is_hidden(index).unwrap_or(false) {
        if let Some(next) = index.checked_add(1) {
            index = next;
        } else {
            break;
        }
    }
    index
}
