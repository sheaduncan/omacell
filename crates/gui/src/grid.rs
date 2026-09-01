//! Virtualized grid painter (fills → gridlines → text → selection → outlines).

use egui::{
    Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2, WidgetInfo, WidgetType, pos2,
};
use omacell_core::addr::{SheetId, col_to_letters};
use omacell_core::geometry::{DEFAULT_COL_PX, DEFAULT_ROW_PX};
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use omacell_core::locale::LocaleId;
use omacell_core::numfmt::{FormatValue, format};
use omacell_core::spill::SpillTable;
use omacell_core::style::{Border, BorderSide, BorderStyle};
use omacell_core::style::{Color, Fill, HorizontalAlign, Style};
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
use omacell_ui::{EditSurface, UiSession, Viewport};

use crate::theme::GuiTheme;

const EDGE_PX: f32 = 4.0;

/// Geometry of the last painted grid, for hit-testing.
#[derive(Clone, Debug)]
pub struct GridLayout {
    /// Grid widget rect in screen points.
    pub rect: Rect,
    /// Row-header width.
    pub header_w: f32,
    /// Column-header height.
    pub header_h: f32,
    /// Fallback cell width in points.
    pub cell_w: f32,
    /// Fallback cell height in points.
    pub cell_h: f32,
    cols: Vec<(u16, f32, f32)>,
    rows: Vec<(u32, f32, f32)>,
    fill_handle: Option<Rect>,
}

impl Default for GridLayout {
    fn default() -> Self {
        Self {
            rect: Rect::ZERO,
            header_w: 48.0,
            header_h: 20.0,
            cell_w: 64.0,
            cell_h: 20.0,
            cols: Vec::new(),
            rows: Vec::new(),
            fill_handle: None,
        }
    }
}

impl GridLayout {
    /// Map a screen position onto a cell, if inside the data or header gutters.
    #[must_use]
    pub fn hit(&self, pos: Pos2, vp: &Viewport) -> Option<(u32, u16)> {
        if !self.rect.contains(pos) {
            return None;
        }
        if let (Some(row), Some(col)) = (self.hit_row(pos.y), self.hit_col(pos.x)) {
            return Some((row, col));
        }
        let local = pos - self.rect.min;
        let y = ((local.y - self.header_h) as f64).max(0.0);
        let x = ((local.x - self.header_w) as f64).max(0.0);
        let row =
            vp.hit_row((y * f64::from(DEFAULT_ROW_PX) / f64::from(self.cell_h.max(1.0))) as u64);
        let col =
            vp.hit_col((x * f64::from(DEFAULT_COL_PX) / f64::from(self.cell_w.max(1.0))) as u64);
        Some((row, col))
    }

    fn hit_row(&self, y: f32) -> Option<u32> {
        self.rows
            .iter()
            .find(|(_, y0, y1)| y >= *y0 && y < *y1)
            .map(|(row, _, _)| *row)
    }

    fn hit_col(&self, x: f32) -> Option<u16> {
        self.cols
            .iter()
            .find(|(_, x0, x1)| x >= *x0 && x < *x1)
            .map(|(col, _, _)| *col)
    }

    /// Whether `pos` is in the row header gutter.
    #[must_use]
    pub fn in_row_header(&self, pos: Pos2) -> bool {
        let local = pos - self.rect.min;
        self.rect.contains(pos) && local.x < self.header_w && local.y >= self.header_h
    }

    /// Whether `pos` is in the column header gutter.
    #[must_use]
    pub fn in_col_header(&self, pos: Pos2) -> bool {
        let local = pos - self.rect.min;
        self.rect.contains(pos) && local.y < self.header_h && local.x >= self.header_w
    }

    /// Column whose right edge is under `pos`, for resize.
    #[must_use]
    pub fn col_edge(&self, pos: Pos2) -> Option<u16> {
        if !self.in_col_header(pos) {
            return None;
        }
        self.cols
            .iter()
            .find(|(_, _, x1)| (pos.x - *x1).abs() <= EDGE_PX)
            .map(|(col, _, _)| *col)
    }

    /// Row whose bottom edge is under `pos`, for resize.
    #[must_use]
    pub fn row_edge(&self, pos: Pos2) -> Option<u32> {
        if !self.in_row_header(pos) {
            return None;
        }
        self.rows
            .iter()
            .find(|(_, _, y1)| (pos.y - *y1).abs() <= EDGE_PX)
            .map(|(row, _, _)| *row)
    }

    /// Whether `pos` is on the fill handle of the cursor cell.
    #[must_use]
    pub fn in_fill_handle(&self, pos: Pos2) -> bool {
        self.fill_handle.is_some_and(|rect| rect.contains(pos))
    }

    /// Screen rect covering `row`/`col` if those cells were painted.
    #[must_use]
    pub fn cell_rect(&self, row: u32, col: u16) -> Option<Rect> {
        let (_, x0, x1) = self.cols.iter().find(|(c, _, _)| *c == col)?;
        let (_, y0, y1) = self.rows.iter().find(|(r, _, _)| *r == row)?;
        Some(Rect::from_min_max(pos2(*x0, *y0), pos2(*x1, *y1)))
    }
}

/// Paint the visible window. Returns layout for the next pointer event.
pub fn paint(
    ui: &mut Ui,
    wb: &Workbook,
    spill: &SpillTable,
    session: &UiSession,
    theme: &GuiTheme,
    ppp: f32,
    a11y_label: &str,
) -> GridLayout {
    let avail = ui.available_size();
    let (rect, _response) = ui.allocate_exact_size(avail, Sense::click_and_drag());
    let zoom = session.viewport().zoom.clamp(0.25, 8.0) as f32;
    let cell_h = snap(DEFAULT_ROW_PX as f32 * zoom, ppp).max(12.0);
    let cell_w = snap(DEFAULT_COL_PX as f32 * zoom, ppp).max(24.0);
    let header_h = cell_h;
    let header_w = 48.0;
    let mut vp = session.viewport();
    let data_h = (rect.height() - header_h).max(1.0);
    let data_w = (rect.width() - header_w).max(1.0);
    vp.height_px = (data_h / zoom).max(1.0) as u32;
    vp.width_px = (data_w / zoom).max(1.0) as u32;
    session.set_viewport(vp.clone());

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme.background);

    let sel = session.selection();
    let sheet = sel.sheet;
    let (first_row, _, _) = vp.screen_rows();
    let first_col = vp.first_col.max(vp.freeze.cols);
    let show_formulas = session.show_formulas();
    let edit = session.edit();
    let grid_lines = session.config().appearance.grid_lines;
    let font = FontId::monospace((theme.ui_font_size_pt as f32 * zoom).clamp(9.0, 48.0));
    let hair = 1.0_f32 / ppp.max(1.0);

    let mut col_x = rect.left() + header_w;
    let mut header_cols: Vec<(u16, f32, f32)> = Vec::new();
    for col in 0..vp.freeze.cols {
        let w = snap(vp.col_px(col) as f32, ppp);
        if w <= 0.0 {
            continue;
        }
        let x0 = snap_pos(col_x, ppp);
        let x1 = snap_pos(col_x + w, ppp);
        header_cols.push((col, x0, x1));
        col_x = x1;
    }
    let freeze_x = col_x;
    let mut col = first_col;
    while col_x < rect.right() && col < MAX_COLS {
        if col < vp.freeze.cols {
            col = col.saturating_add(1);
            continue;
        }
        let w = snap(vp.col_px(col) as f32, ppp);
        if w <= 0.0 {
            col = col.saturating_add(1);
            continue;
        }
        let x0 = snap_pos(col_x, ppp);
        let x1 = snap_pos(col_x + w, ppp);
        header_cols.push((col, x0, x1));
        col_x = x1;
        col = col.saturating_add(1);
    }

    for (col, x0, x1) in &header_cols {
        let label = col_to_letters(*col).unwrap_or_else(|_| "?".into());
        painter.rect_filled(
            Rect::from_min_max(pos2(*x0, rect.top()), pos2(*x1, rect.top() + header_h)),
            0.0,
            theme.header_background,
        );
        painter.text(
            pos2((*x0 + *x1) * 0.5, rect.top() + header_h * 0.5),
            Align2::CENTER_CENTER,
            label,
            font.clone(),
            theme.header_foreground,
        );
    }

    let mut row_list: Vec<u32> = Vec::new();
    for row in 0..vp.freeze.rows {
        if vp.row_px(row) != 0 {
            row_list.push(row);
        }
    }
    let mut row = first_row;
    while row < MAX_ROWS && row_list.len() < 512 {
        if row >= vp.freeze.rows && vp.row_px(row) != 0 {
            row_list.push(row);
        }
        let next = row.saturating_add(1);
        if next <= row {
            break;
        }
        row = next;
        if (row_list.len() as f32) * cell_h > data_h + header_h {
            break;
        }
    }

    let active = sel.active();
    let review = session.changeset_review();
    let review_sheet = wb.sheet(sheet).map(|sheet| sheet.name.as_str());
    let (r0, c0, r1, c1) = active.normalized();
    let last_visible_col = header_cols.last().map(|(col, _, _)| *col);
    let last_visible_row = row_list.last().copied();
    let mut painted_rows: Vec<(u32, f32, f32)> = Vec::new();
    let mut fill_handle = None;
    let mut cursor_rect = None;
    let mut row_y = rect.top() + header_h;
    for row in row_list {
        let h = snap(vp.row_px(row) as f32, ppp);
        if h <= 0.0 {
            continue;
        }
        let y0 = snap_pos(row_y, ppp);
        let y1 = snap_pos(row_y + h, ppp);
        painted_rows.push((row, y0, y1));
        painter.rect_filled(
            Rect::from_min_max(pos2(rect.left(), y0), pos2(rect.left() + header_w, y1)),
            0.0,
            theme.header_background,
        );
        painter.text(
            pos2(rect.left() + header_w - 4.0, (y0 + y1) * 0.5),
            Align2::RIGHT_CENTER,
            format!("{}", row + 1),
            font.clone(),
            theme.header_foreground,
        );
        for (col, x0, x1) in &header_cols {
            let cell_rect = Rect::from_min_max(pos2(*x0, y0), pos2(*x1, y1));
            let style = wb
                .get(sheet, row, *col)
                .ok()
                .flatten()
                .and_then(|slot| wb.intern().styles.get(slot.style));
            if let Some(fill) = style.and_then(style_fill) {
                painter.rect_filled(cell_rect, 0.0, fill);
            }
            let in_sel = row >= r0 && row <= r1 && *col >= c0 && *col <= c1;
            let is_cursor = sel.cursor.row == row && sel.cursor.col == *col;
            if in_sel {
                painter.rect_filled(cell_rect, 0.0, theme.selection.gamma_multiply(0.45));
            }
            if let Some(mark) = review
                .as_ref()
                .and_then(|review| review_sheet.and_then(|name| review.cell_mark(name, row, *col)))
            {
                let color = if mark.accepted {
                    theme.success
                } else {
                    theme.error
                };
                painter.rect_filled(cell_rect, 0.0, color.gamma_multiply(0.32));
                if mark.selected {
                    painter.rect_stroke(
                        cell_rect.shrink(1.0),
                        0.0,
                        Stroke::new(2.0_f32, theme.warning),
                        egui::StrokeKind::Inside,
                    );
                }
            }
            let editing_here = edit.surface == EditSurface::InCell
                && edit.origin.is_some_and(|origin| {
                    origin.sheet.unwrap_or(sheet) == sheet
                        && origin.row == row
                        && origin.col == *col
                });
            let (text, align, is_error, stale) = if editing_here {
                (
                    format!(
                        "{}{}",
                        edit.buffer,
                        edit.ghost.as_deref().unwrap_or_default()
                    ),
                    Align::Left,
                    false,
                    false,
                )
            } else {
                cell_text(wb, sheet, row, *col, show_formulas)
            };
            let mut color = theme.foreground;
            if is_error {
                color = theme.error;
            } else if let Some(file_color) =
                style.and_then(|style| explicit_color(style.font.color))
            {
                color = file_color;
            }
            if stale {
                painter.rect_filled(cell_rect, 0.0, theme.stale.gamma_multiply(0.35));
            }
            if let Some(region) = spill.region_at(sheet, row, *col) {
                let last_row = region
                    .origin
                    .row
                    .saturating_add(region.rows.saturating_sub(1));
                let last_col = region.origin.col.saturating_add(
                    u16::try_from(region.cols.saturating_sub(1)).unwrap_or(u16::MAX),
                );
                let on_edge = row == region.origin.row
                    || *col == region.origin.col
                    || row == last_row
                    || *col == last_col;
                if on_edge {
                    painter.rect_stroke(
                        cell_rect.shrink(0.5),
                        0.0,
                        Stroke::new(hair, theme.warning),
                        egui::StrokeKind::Inside,
                    );
                }
            }
            let galley_pos = match align {
                Align::Left => pos2(cell_rect.left() + 3.0, cell_rect.center().y),
                Align::Right => pos2(cell_rect.right() - 3.0, cell_rect.center().y),
                Align::Center => cell_rect.center(),
            };
            let anchor = match align {
                Align::Left => Align2::LEFT_CENTER,
                Align::Right => Align2::RIGHT_CENTER,
                Align::Center => Align2::CENTER_CENTER,
            };
            let cell_font = style.map_or_else(
                || font.clone(),
                |style| FontId::monospace((style.font.size_pt as f32 * zoom).clamp(9.0, 72.0)),
            );
            painter.text(galley_pos, anchor, text, cell_font, color);
            if grid_lines {
                let stroke = Stroke::new(hair, theme.grid_line);
                painter.line_segment([pos2(*x0, y0), pos2(*x0, y1)], stroke);
                painter.line_segment([pos2(*x0, y0), pos2(*x1, y0)], stroke);
            }
            let borders = cell_border(wb, sheet, row, *col);
            let left = strongest_border(
                borders.left,
                col.checked_sub(1)
                    .map(|neighbor| cell_border(wb, sheet, row, neighbor).right)
                    .unwrap_or_default(),
            );
            let top = strongest_border(
                borders.top,
                row.checked_sub(1)
                    .map(|neighbor| cell_border(wb, sheet, neighbor, *col).bottom)
                    .unwrap_or_default(),
            );
            draw_border(
                &painter,
                [pos2(*x0, y0), pos2(*x0, y1)],
                left,
                theme.foreground,
                ppp,
            );
            draw_border(
                &painter,
                [pos2(*x0, y0), pos2(*x1, y0)],
                top,
                theme.foreground,
                ppp,
            );
            if Some(*col) == last_visible_col {
                let right = strongest_border(
                    borders.right,
                    col.checked_add(1)
                        .filter(|neighbor| *neighbor < MAX_COLS)
                        .map(|neighbor| cell_border(wb, sheet, row, neighbor).left)
                        .unwrap_or_default(),
                );
                draw_border(
                    &painter,
                    [pos2(*x1, y0), pos2(*x1, y1)],
                    right,
                    theme.foreground,
                    ppp,
                );
            }
            if Some(row) == last_visible_row {
                let bottom = strongest_border(
                    borders.bottom,
                    row.checked_add(1)
                        .filter(|neighbor| *neighbor < MAX_ROWS)
                        .map(|neighbor| cell_border(wb, sheet, neighbor, *col).top)
                        .unwrap_or_default(),
                );
                draw_border(
                    &painter,
                    [pos2(*x0, y1), pos2(*x1, y1)],
                    bottom,
                    theme.foreground,
                    ppp,
                );
            }
            if is_cursor {
                painter.rect_stroke(
                    cell_rect.shrink(0.5),
                    0.0,
                    Stroke::new(2.0_f32, theme.cursor),
                    egui::StrokeKind::Inside,
                );
                let handle = Rect::from_min_size(
                    pos2(cell_rect.right() - 5.0, cell_rect.bottom() - 5.0),
                    Vec2::splat(6.0),
                );
                painter.rect_filled(handle, 0.0, theme.cursor);
                fill_handle = Some(handle);
                cursor_rect = Some(cell_rect);
            }
        }
        if vp.freeze.rows > 0 && row + 1 == vp.freeze.rows {
            painter.line_segment(
                [pos2(rect.left(), y1), pos2(rect.right(), y1)],
                Stroke::new(2.0_f32, theme.frozen_edge),
            );
        }
        row_y = y1;
        if row_y > rect.bottom() {
            break;
        }
    }

    if vp.freeze.cols > 0 {
        painter.line_segment(
            [pos2(freeze_x, rect.top()), pos2(freeze_x, rect.bottom())],
            Stroke::new(2.0_f32, theme.frozen_edge),
        );
    }

    if let Some(fr) = cursor_rect {
        let id = ui.id().with("focused-cell");
        let cell_resp = ui.interact(fr, id, Sense::focusable_noninteractive());
        cell_resp.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, a11y_label));
        let no_widget_has_focus = ui.ctx().memory(|memory| memory.focused().is_none());
        if !cell_resp.has_focus() && no_widget_has_focus {
            cell_resp.request_focus();
        }
    }

    GridLayout {
        rect,
        header_w,
        header_h,
        cell_w,
        cell_h,
        cols: header_cols,
        rows: painted_rows,
        fill_handle,
    }
}

fn explicit_color(color: Color) -> Option<Color32> {
    let Color::Rgb { argb } = color else {
        return None;
    };
    let [a, r, g, b] = argb.to_be_bytes();
    Some(Color32::from_rgba_unmultiplied(r, g, b, a))
}

fn style_fill(style: &Style) -> Option<Color32> {
    match &style.fill {
        Fill::Solid { fg } => explicit_color(*fg),
        Fill::Pattern { fg, bg, .. } => explicit_color(*bg).or_else(|| explicit_color(*fg)),
        Fill::Gradient(gradient) => gradient
            .stops
            .first()
            .and_then(|stop| explicit_color(stop.color)),
        Fill::None => None,
    }
}

fn cell_border(wb: &Workbook, sheet: SheetId, row: u32, col: u16) -> Border {
    wb.get(sheet, row, col)
        .ok()
        .flatten()
        .and_then(|slot| wb.intern().styles.get(slot.style))
        .map(|style| style.border)
        .unwrap_or_default()
}

fn strongest_border(first: BorderSide, second: BorderSide) -> BorderSide {
    if border_rank(second.style) > border_rank(first.style) {
        second
    } else {
        first
    }
}

fn border_rank(style: BorderStyle) -> u8 {
    match style {
        BorderStyle::None => 0,
        BorderStyle::Hair => 1,
        BorderStyle::Dotted => 2,
        BorderStyle::DashDotDot => 3,
        BorderStyle::DashDot | BorderStyle::Dashed => 4,
        BorderStyle::Thin => 5,
        BorderStyle::MediumDashDotDot => 6,
        BorderStyle::MediumDashDot | BorderStyle::MediumDashed => 7,
        BorderStyle::Medium | BorderStyle::SlantDashDot => 8,
        BorderStyle::Thick => 9,
        BorderStyle::Double => 10,
    }
}

fn draw_border(
    painter: &egui::Painter,
    points: [Pos2; 2],
    side: BorderSide,
    fallback: Color32,
    ppp: f32,
) {
    if side.style == BorderStyle::None {
        return;
    }
    let color = explicit_color(side.color).unwrap_or(fallback);
    let hair = 1.0 / ppp.max(1.0);
    let width = match side.style {
        BorderStyle::Hair => hair,
        BorderStyle::Medium
        | BorderStyle::MediumDashed
        | BorderStyle::MediumDashDot
        | BorderStyle::MediumDashDotDot
        | BorderStyle::SlantDashDot => 2.0,
        BorderStyle::Thick | BorderStyle::Double => 3.0,
        _ => 1.0,
    };
    let stroke = Stroke::new(width, color);
    match side.style {
        BorderStyle::Dotted => painter.extend(egui::Shape::dotted_line(
            &points,
            color,
            2.0 * width,
            width * 0.5,
        )),
        BorderStyle::Dashed
        | BorderStyle::MediumDashed
        | BorderStyle::DashDot
        | BorderStyle::MediumDashDot
        | BorderStyle::DashDotDot
        | BorderStyle::MediumDashDotDot
        | BorderStyle::SlantDashDot => painter.extend(egui::Shape::dashed_line(
            &points,
            stroke,
            4.0 * width,
            2.0 * width,
        )),
        BorderStyle::Double => {
            let delta = points[1] - points[0];
            let normal = if delta.x.abs() > delta.y.abs() {
                egui::vec2(0.0, 1.0)
            } else {
                egui::vec2(1.0, 0.0)
            };
            painter.line_segment(
                [points[0] - normal, points[1] - normal],
                Stroke::new(1.0_f32, color),
            );
            painter.line_segment(
                [points[0] + normal, points[1] + normal],
                Stroke::new(1.0_f32, color),
            );
        }
        _ => {
            painter.line_segment(points, stroke);
        }
    }
}

fn snap(v: f32, ppp: f32) -> f32 {
    if ppp <= 0.0 {
        return v;
    }
    (v * ppp).round() / ppp
}

fn snap_pos(v: f32, ppp: f32) -> f32 {
    snap(v, ppp)
}

#[derive(Clone, Copy)]
enum Align {
    Left,
    Right,
    Center,
}

fn cell_text(
    wb: &Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
    formulas: bool,
) -> (String, Align, bool, bool) {
    let Ok(Some(slot)) = wb.get(sheet, row, col) else {
        return (String::new(), Align::Left, false, false);
    };
    let stale = slot.flags.stale();
    if formulas && let Some(fid) = slot.formula {
        let src = wb.intern().formulas.get(fid).unwrap_or("");
        return (src.to_string(), Align::Left, false, stale);
    }
    let (text, align, err) = match slot.value {
        Value::Empty => (String::new(), Align::Left, false),
        Value::Number(n) => {
            let code = wb
                .intern()
                .styles
                .get(slot.style)
                .map(|s| s.num_fmt)
                .and_then(|id| wb.num_fmt_code(id))
                .unwrap_or_else(|| "General".into());
            let formatted = format(FormatValue::Number(n), code.as_ref(), LocaleId::EN_US);
            (formatted.text, Align::Right, false)
        }
        Value::Bool(true) => ("TRUE".into(), Align::Left, false),
        Value::Bool(false) => ("FALSE".into(), Align::Left, false),
        Value::Text(id) => (
            wb.intern().strings.get(id).unwrap_or("").to_string(),
            Align::Left,
            false,
        ),
        Value::Error(kind) => (
            format!("#{}", kind.as_str().trim_start_matches('#')),
            Align::Left,
            true,
        ),
        Value::Array(_) => (String::new(), Align::Left, false),
    };
    let align = wb
        .intern()
        .styles
        .get(slot.style)
        .map(|style| match style.alignment.horizontal {
            HorizontalAlign::Left | HorizontalAlign::Fill | HorizontalAlign::Justify => Align::Left,
            HorizontalAlign::Right => Align::Right,
            HorizontalAlign::Center
            | HorizontalAlign::CenterContinuous
            | HorizontalAlign::Distributed => Align::Center,
            HorizontalAlign::General => align,
        })
        .unwrap_or(align);
    (text, align, err, stale)
}

/// Formula-bar text for the active cell.
#[must_use]
pub fn formula_text(wb: &Workbook, session: &UiSession) -> String {
    let edit = session.edit();
    if !edit.is_idle() {
        return edit.buffer.clone();
    }
    let sel = session.selection();
    let Ok(Some(slot)) = wb.get(sel.sheet, sel.cursor.row, sel.cursor.col) else {
        return String::new();
    };
    if let Some(fid) = slot.formula {
        return wb.intern().formulas.get(fid).unwrap_or("").to_string();
    }
    cell_text(wb, sel.sheet, sel.cursor.row, sel.cursor.col, false).0
}

/// AccessKit / status label for the focused cell.
#[must_use]
pub fn cell_a11y(wb: &Workbook, session: &UiSession) -> String {
    let sel = session.selection();
    let addr = format!(
        "{}{}",
        col_to_letters(sel.cursor.col).unwrap_or_else(|_| "A".into()),
        sel.cursor.row + 1
    );
    let Ok(slot) = wb.get(sel.sheet, sel.cursor.row, sel.cursor.col) else {
        return format!("cell {addr} empty");
    };
    let Some(slot) = slot else {
        return format!("cell {addr} empty");
    };
    let (text, _, is_error, _) = cell_text(wb, sel.sheet, sel.cursor.row, sel.cursor.col, false);
    let formula = slot
        .formula
        .and_then(|id| wb.intern().formulas.get(id).map(str::to_string));
    let mut out = format!(
        "cell {addr} value {}",
        if text.is_empty() { "empty" } else { &text }
    );
    if let Some(formula) = formula {
        out.push_str(" formula ");
        out.push_str(&formula);
    }
    if is_error {
        out.push_str(" error");
    }
    out
}

/// Estimate a fitted column width in CSS pixels at zoom 1.
#[must_use]
pub fn autofit_col_px(wb: &Workbook, session: &UiSession, col: u16) -> u32 {
    let sel = session.selection();
    let vp = session.viewport();
    let (first, _, last) = vp.screen_rows();
    let mut max_chars = col_to_letters(col).map(|s| s.len()).unwrap_or(1);
    for row in first..=last.max(first.saturating_add(40)) {
        let (text, _, _, _) = cell_text(wb, sel.sheet, row, col, session.show_formulas());
        max_chars = max_chars.max(text.chars().count());
    }
    ((max_chars as u32).saturating_add(2))
        .saturating_mul(8)
        .clamp(24, 480)
}

#[cfg(test)]
mod tests {
    use super::{GridLayout, explicit_color, strongest_border, style_fill};
    use egui::{Color32, Rect, pos2};
    use omacell_core::style::{BorderSide, BorderStyle, Color, Fill, Style};
    use omacell_ui::Viewport;

    #[test]
    fn hit_uses_painted_stops() {
        let layout = GridLayout {
            rect: Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(200.0, 80.0)),
            header_w: 48.0,
            header_h: 20.0,
            cell_w: 64.0,
            cell_h: 20.0,
            cols: vec![(0, 48.0, 112.0), (1, 112.0, 176.0)],
            rows: vec![(0, 20.0, 40.0), (1, 40.0, 60.0)],
            fill_handle: None,
        };
        let vp = Viewport::default();
        assert_eq!(layout.hit(pos2(50.0, 25.0), &vp), Some((0, 0)));
        assert_eq!(layout.hit(pos2(120.0, 45.0), &vp), Some((1, 1)));
        assert!(layout.in_col_header(pos2(80.0, 10.0)));
        assert_eq!(layout.col_edge(pos2(112.0, 10.0)), Some(0));
    }

    #[test]
    fn explicit_file_colors_are_not_rethemed() {
        let red = Color::Rgb { argb: 0x80FF_0000 };
        assert_eq!(
            explicit_color(red),
            Some(Color32::from_rgba_unmultiplied(255, 0, 0, 128))
        );
        let style = Style {
            fill: Fill::Solid { fg: red },
            ..Style::default()
        };
        assert_eq!(
            style_fill(&style),
            Some(Color32::from_rgba_unmultiplied(255, 0, 0, 128))
        );
        assert_eq!(explicit_color(Color::Auto), None);
    }

    #[test]
    fn shared_edges_use_the_stronger_excel_border() {
        let thin = BorderSide {
            style: BorderStyle::Thin,
            color: Color::Auto,
        };
        let thick = BorderSide {
            style: BorderStyle::Thick,
            color: Color::Auto,
        };
        assert_eq!(strongest_border(thin, thick), thick);
        assert_eq!(strongest_border(thick, thin), thick);
    }
}
