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
        if zoom.is_finite() {
            self.zoom = zoom.clamp(0.25, 8.0);
        }
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
        hit_axis(
            &self.rows,
            y_px,
            self.zoom,
            self.freeze.rows,
            self.first_row.max(self.freeze.rows),
        )
    }

    /// Hit-test a grid-local pixel to a column index, skipping hidden columns.
    #[must_use]
    pub fn hit_col(&self, x_px: u64) -> u16 {
        hit_axis(
            &self.cols,
            x_px,
            self.zoom,
            u32::from(self.freeze.cols),
            u32::from(self.first_col.max(self.freeze.cols)),
        ) as u16
    }

    /// Scroll so `row` is inside the data window.
    pub fn ensure_row_visible(&mut self, row: u32) {
        if row < self.freeze.rows {
            return;
        }
        self.first_row = self.first_row.max(self.freeze.rows);
        if row < self.first_row {
            self.first_row = row;
        } else if row
            > visible_end(
                &self.rows,
                self.first_row,
                self.height_px,
                self.zoom,
                self.freeze.rows,
            )
        {
            self.first_row =
                scroll_start_for(&self.rows, row, self.height_px, self.zoom, self.freeze.rows);
        }
    }

    /// Scroll so `col` is inside the data window.
    pub fn ensure_col_visible(&mut self, col: u16) {
        if col < self.freeze.cols {
            return;
        }
        self.first_col = self.first_col.max(self.freeze.cols);
        if col < self.first_col {
            self.first_col = col;
        } else if u32::from(col)
            > visible_end(
                &self.cols,
                u32::from(self.first_col),
                self.width_px,
                self.zoom,
                u32::from(self.freeze.cols),
            )
        {
            let first = scroll_start_for(
                &self.cols,
                u32::from(col),
                self.width_px,
                self.zoom,
                u32::from(self.freeze.cols),
            );
            self.first_col = u16::try_from(first).unwrap_or(u16::MAX);
        }
    }

    /// Center a cell in the non-frozen data viewport.
    pub fn center_on(&mut self, row: u32, col: u16) {
        if row >= self.freeze.rows {
            self.first_row =
                center_start_for(&self.rows, row, self.height_px, self.zoom, self.freeze.rows);
        }
        if col >= self.freeze.cols {
            let first = center_start_for(
                &self.cols,
                u32::from(col),
                self.width_px,
                self.zoom,
                u32::from(self.freeze.cols),
            );
            self.first_col = u16::try_from(first).unwrap_or(u16::MAX);
        }
    }

    /// Number of rows in the scrolling portion of the current viewport.
    #[must_use]
    pub fn page_rows(&self) -> u32 {
        let first = self.first_row.max(self.freeze.rows);
        visible_end(
            &self.rows,
            first,
            self.height_px,
            self.zoom,
            self.freeze.rows,
        )
        .saturating_sub(first)
        .saturating_add(1)
    }

    /// Number of columns in the scrolling portion of the current viewport.
    #[must_use]
    pub fn page_cols(&self) -> u16 {
        let first = u32::from(self.first_col.max(self.freeze.cols));
        let count = visible_end(
            &self.cols,
            first,
            self.width_px,
            self.zoom,
            u32::from(self.freeze.cols),
        )
        .saturating_sub(first)
        .saturating_add(1);
        u16::try_from(count).unwrap_or(u16::MAX)
    }

    /// Top, middle, and bottom rows of the scrolling data window.
    #[must_use]
    pub fn screen_rows(&self) -> (u32, u32, u32) {
        let first = self.first_row.max(self.freeze.rows);
        let last = visible_end(
            &self.rows,
            first,
            self.height_px,
            self.zoom,
            self.freeze.rows,
        );
        (first, midpoint(&self.rows, first, last), last)
    }
}

fn scaled(px: u32, zoom: f64) -> u32 {
    ((f64::from(px) * zoom).round() as u32).max(if px == 0 { 0 } else { 1 })
}

fn hit_axis(axis: &AxisGeometry, screen_px: u64, zoom: f64, frozen: u32, first: u32) -> u32 {
    let zoom = zoom.max(0.25);
    let frozen_screen_px = (axis.index_to_pixel(frozen) as f64 * zoom).round() as u64;
    if screen_px < frozen_screen_px {
        return axis.pixel_to_index((screen_px as f64 / zoom) as u64);
    }
    let data_screen_px = screen_px.saturating_sub(frozen_screen_px);
    let data_axis_px = (data_screen_px as f64 / zoom) as u64;
    axis.pixel_to_index(axis.index_to_pixel(first).saturating_add(data_axis_px))
}

fn skip_hidden_row(rows: &AxisGeometry, mut index: u32) -> u32 {
    while index + 1 < rows.len() && rows.is_hidden(index).unwrap_or(false) {
        if let Some(next) = index.checked_add(1) {
            index = next;
        } else {
            break;
        }
    }
    index
}

fn data_window_px(axis: &AxisGeometry, viewport_px: u32, zoom: f64, frozen: u32) -> u64 {
    let frozen_px = axis.index_to_pixel(frozen) as f64 * zoom;
    let available = (f64::from(viewport_px) - frozen_px).max(1.0);
    (available / zoom.max(0.25)).max(1.0) as u64
}

fn visible_end(axis: &AxisGeometry, first: u32, viewport_px: u32, zoom: f64, frozen: u32) -> u32 {
    let start = axis.index_to_pixel(first);
    let span = data_window_px(axis, viewport_px, zoom, frozen);
    axis.pixel_to_index(start.saturating_add(span.saturating_sub(1)))
}

fn scroll_start_for(
    axis: &AxisGeometry,
    target: u32,
    viewport_px: u32,
    zoom: f64,
    frozen: u32,
) -> u32 {
    let span = data_window_px(axis, viewport_px, zoom, frozen);
    let target_end = axis.index_to_pixel(target.saturating_add(1));
    axis.pixel_to_index(target_end.saturating_sub(span))
        .max(frozen)
}

fn center_start_for(
    axis: &AxisGeometry,
    target: u32,
    viewport_px: u32,
    zoom: f64,
    frozen: u32,
) -> u32 {
    let span = data_window_px(axis, viewport_px, zoom, frozen);
    let target_px = axis.index_to_pixel(target);
    axis.pixel_to_index(target_px.saturating_sub(span / 2))
        .max(frozen)
}

fn midpoint(axis: &AxisGeometry, first: u32, last: u32) -> u32 {
    let start_px = axis.index_to_pixel(first);
    let end_px = axis.index_to_pixel(last.saturating_add(1));
    axis.pixel_to_index(start_px.saturating_add(end_px.saturating_sub(start_px) / 2))
}
