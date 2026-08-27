//! Virtualized 1,048,576 × 50 grid painted with a custom `Painter`.

use egui::{FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2, WidgetInfo, WidgetType, pos2, vec2};

use crate::cache::ShapeCache;
use crate::theme::Palette;

pub const N_ROWS: u32 = 1_048_576;
pub const N_COLS: u32 = 50;
pub const ROW_H: f32 = 22.0;
pub const COL_W: f32 = 88.0;
pub const HEADER_H: f32 = 24.0;
pub const ROW_HEAD_W: f32 = 72.0;

#[derive(Clone, Debug)]
pub struct GridState {
    pub scroll: Vec2,
    pub focused: (u32, u32),
    pub font_size: f32,
}

impl Default for GridState {
    fn default() -> Self {
        Self {
            scroll: Vec2::ZERO,
            focused: (0, 0),
            font_size: 13.0,
        }
    }
}

pub struct PaintStats {
    pub vis_rows: u32,
    pub vis_cols: u32,
    pub shaped: usize,
}

pub fn show(
    ui: &mut Ui,
    state: &mut GridState,
    theme: &Palette,
    cache: &mut ShapeCache,
    announce_focus: bool,
) -> PaintStats {
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let ppp = ui.ctx().pixels_per_point();

    if response.dragged() {
        state.scroll -= response.drag_delta();
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta);
        state.scroll -= scroll;
    }
    clamp_scroll(state, rect);

    let body = Rect::from_min_max(
        pos2(rect.min.x + ROW_HEAD_W, rect.min.y + HEADER_H),
        rect.max,
    );
    let vis_rows = ((body.height() / ROW_H).ceil() as u32)
        .saturating_add(1)
        .min(N_ROWS);
    let vis_cols = ((body.width() / COL_W).ceil() as u32)
        .saturating_add(1)
        .min(N_COLS);
    let first_row = (state.scroll.y / ROW_H).floor().max(0.0) as u32;
    let first_col = (state.scroll.x / COL_W).floor().max(0.0) as u32;
    let first_row = first_row.min(N_ROWS.saturating_sub(1));
    let first_col = first_col.min(N_COLS.saturating_sub(1));
    let last_row = (first_row + vis_rows).min(N_ROWS);
    let last_col = (first_col + vis_cols).min(N_COLS);

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme.background);

    let font = FontId::monospace(state.font_size);
    let header_font = FontId::monospace(state.font_size - 1.0);
    let hair = 1.0 / ppp;
    let stroke = Stroke::new(hair, theme.grid_line);
    let origin_x = body.min.x - (state.scroll.x % COL_W);
    let origin_y = body.min.y - (state.scroll.y % ROW_H);

    // Body fills (virtualized).
    for row in first_row..last_row {
        let y = snap(origin_y + (row - first_row) as f32 * ROW_H, ppp);
        for col in first_col..last_col {
            let x = snap(origin_x + (col - first_col) as f32 * COL_W, ppp);
            let cell = Rect::from_min_size(pos2(x, y), vec2(COL_W, ROW_H));
            if !cell.intersects(body) {
                continue;
            }
            let fill = if (row, col) == state.focused {
                theme.selection
            } else {
                theme.surface
            };
            painter.rect_filled(cell.intersect(body), 0.0, fill);
        }
    }

    // Body text from the shaping cache.
    for row in first_row..last_row {
        let y = snap(origin_y + (row - first_row) as f32 * ROW_H, ppp);
        for col in first_col..last_col {
            let x = snap(origin_x + (col - first_col) as f32 * COL_W, ppp);
            let cell = Rect::from_min_size(pos2(x, y), vec2(COL_W, ROW_H)).intersect(body);
            if cell.width() < 2.0 || cell.height() < 2.0 {
                continue;
            }
            let text = cell_text(row, col);
            let galley = cache.galley(ui.ctx(), text, font.clone(), theme.foreground, ppp);
            let pos = pos2(
                cell.min.x + 6.0,
                cell.min.y + (ROW_H - galley.size().y) * 0.5,
            );
            painter
                .with_clip_rect(cell)
                .galley(pos, galley, theme.foreground);
        }
    }

    // Gridlines, snapped to device pixels so they stay one physical pixel.
    for col in first_col..=last_col {
        let x = line_snap(origin_x + (col - first_col) as f32 * COL_W, ppp);
        if x >= body.min.x && x <= body.max.x {
            painter.line_segment([pos2(x, body.min.y), pos2(x, body.max.y)], stroke);
        }
    }
    for row in first_row..=last_row {
        let y = line_snap(origin_y + (row - first_row) as f32 * ROW_H, ppp);
        if y >= body.min.y && y <= body.max.y {
            painter.line_segment([pos2(body.min.x, y), pos2(body.max.x, y)], stroke);
        }
    }

    // Frozen header row (column letters) — does not scroll vertically.
    let header = Rect::from_min_max(pos2(body.min.x, rect.min.y), pos2(rect.max.x, body.min.y));
    painter.rect_filled(header, 0.0, theme.header_background);
    for col in first_col..last_col {
        let x = snap(origin_x + (col - first_col) as f32 * COL_W, ppp);
        let cell =
            Rect::from_min_size(pos2(x, header.min.y), vec2(COL_W, HEADER_H)).intersect(header);
        if cell.width() < 2.0 {
            continue;
        }
        let label = col_name(col);
        let galley = cache.galley(
            ui.ctx(),
            label,
            header_font.clone(),
            theme.header_foreground,
            ppp,
        );
        let pos = pos2(
            cell.min.x + (cell.width() - galley.size().x) * 0.5,
            cell.min.y + (HEADER_H - galley.size().y) * 0.5,
        );
        painter
            .with_clip_rect(cell)
            .galley(pos, galley, theme.header_foreground);
        let xline = line_snap(x, ppp);
        painter.line_segment(
            [pos2(xline, header.min.y), pos2(xline, header.max.y)],
            stroke,
        );
    }
    painter.line_segment(
        [
            pos2(header.min.x, line_snap(header.max.y, ppp)),
            pos2(header.max.x, line_snap(header.max.y, ppp)),
        ],
        Stroke::new(hair, theme.frozen_edge),
    );

    // Frozen row-index column — does not scroll horizontally.
    let row_head = Rect::from_min_max(pos2(rect.min.x, body.min.y), pos2(body.min.x, rect.max.y));
    painter.rect_filled(row_head, 0.0, theme.header_background);
    for row in first_row..last_row {
        let y = snap(origin_y + (row - first_row) as f32 * ROW_H, ppp);
        let cell = Rect::from_min_size(pos2(row_head.min.x, y), vec2(ROW_HEAD_W, ROW_H))
            .intersect(row_head);
        if cell.height() < 2.0 {
            continue;
        }
        let label = (row + 1).to_string();
        let galley = cache.galley(
            ui.ctx(),
            label,
            header_font.clone(),
            theme.header_foreground,
            ppp,
        );
        let pos = pos2(
            cell.max.x - galley.size().x - 8.0,
            cell.min.y + (ROW_H - galley.size().y) * 0.5,
        );
        painter
            .with_clip_rect(cell)
            .galley(pos, galley, theme.header_foreground);
        let yline = line_snap(y, ppp);
        painter.line_segment(
            [pos2(row_head.min.x, yline), pos2(row_head.max.x, yline)],
            stroke,
        );
    }
    painter.line_segment(
        [
            pos2(line_snap(row_head.max.x, ppp), row_head.min.y),
            pos2(line_snap(row_head.max.x, ppp), row_head.max.y),
        ],
        Stroke::new(hair, theme.frozen_edge),
    );

    // Corner.
    let corner = Rect::from_min_max(rect.min, pos2(body.min.x, body.min.y));
    painter.rect_filled(corner, 0.0, theme.header_background);

    // Focused-cell AccessKit node (the Orca target). One node, not every cell.
    let focused_rect = cell_rect(
        state.focused,
        origin_x,
        origin_y,
        first_row,
        first_col,
        body,
        ppp,
    );
    if let Some(fr) = focused_rect {
        painter.rect_stroke(
            fr,
            0.0,
            Stroke::new(hair * 2.0, theme.cursor),
            egui::StrokeKind::Inside,
        );
        let id = ui.id().with("focused-cell");
        let cell_resp = ui.interact(fr, id, Sense::focusable_noninteractive());
        let addr = a1(state.focused.0, state.focused.1);
        let value = cell_text(state.focused.0, state.focused.1);
        cell_resp.widget_info(|| {
            WidgetInfo::labeled(
                WidgetType::Label,
                true,
                format!("cell {addr} value {value}"),
            )
        });
        if announce_focus {
            cell_resp.request_focus();
        }
    }

    if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            if body.contains(pos) {
                let col = first_col + ((pos.x - origin_x) / COL_W).floor().max(0.0) as u32;
                let row = first_row + ((pos.y - origin_y) / ROW_H).floor().max(0.0) as u32;
                state.focused = (row.min(N_ROWS - 1), col.min(N_COLS - 1));
            }
        }
    }

    PaintStats {
        vis_rows: last_row.saturating_sub(first_row),
        vis_cols: last_col.saturating_sub(first_col),
        shaped: cache.len(),
    }
}

pub fn jump_to(state: &mut GridState, row: u32, col: u32) {
    state.focused = (row.min(N_ROWS - 1), col.min(N_COLS - 1));
    state.scroll.y = state.focused.0 as f32 * ROW_H;
    state.scroll.x = state.focused.1 as f32 * COL_W;
}

pub fn a1(row: u32, col: u32) -> String {
    format!("{}{}", col_name(col), row + 1)
}

fn cell_text(row: u32, col: u32) -> String {
    // Deterministic synthetic values; nothing is stored per cell.
    if col == 0 {
        a1(row, col)
    } else if col == 1 {
        format!("{:.2}", (row as f64) * 0.25)
    } else {
        format!("{}{row}", col_name(col))
    }
}

fn col_name(mut col: u32) -> String {
    let mut s = String::new();
    col += 1;
    while col > 0 {
        col -= 1;
        s.insert(0, (b'A' + (col % 26) as u8) as char);
        col /= 26;
    }
    s
}

fn snap(x: f32, ppp: f32) -> f32 {
    (x * ppp).round() / ppp
}

fn line_snap(x: f32, ppp: f32) -> f32 {
    // Center of a physical pixel so a 1-device-px stroke is crisp.
    (x * ppp).floor() / ppp + 0.5 / ppp
}

fn clamp_scroll(state: &mut GridState, rect: Rect) {
    let max_x = (N_COLS as f32 * COL_W - (rect.width() - ROW_HEAD_W)).max(0.0);
    let max_y = (N_ROWS as f32 * ROW_H - (rect.height() - HEADER_H)).max(0.0);
    state.scroll.x = state.scroll.x.clamp(0.0, max_x);
    state.scroll.y = state.scroll.y.clamp(0.0, max_y);
}

fn cell_rect(
    focused: (u32, u32),
    origin_x: f32,
    origin_y: f32,
    first_row: u32,
    first_col: u32,
    body: Rect,
    ppp: f32,
) -> Option<Rect> {
    if focused.0 < first_row || focused.1 < first_col {
        return None;
    }
    let x = snap(origin_x + (focused.1 - first_col) as f32 * COL_W, ppp);
    let y = snap(origin_y + (focused.0 - first_row) as f32 * ROW_H, ppp);
    let r = Rect::from_min_size(Pos2::new(x, y), vec2(COL_W, ROW_H)).intersect(body);
    if r.width() < 2.0 || r.height() < 2.0 {
        None
    } else {
        Some(r)
    }
}
