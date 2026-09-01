//! Virtualized grid, formula bar, tabs, status, palette, and panels.

use omacell_bus::ConditionalFormatSnapshot;
use omacell_core::addr::{SheetId, col_to_letters};
use omacell_core::condfmt::{CfOverlay, CfVisual};
use omacell_core::geometry::{AxisGeometry, DEFAULT_COL_PX, DEFAULT_ROW_PX};
use omacell_core::locale::LocaleId;
use omacell_core::numfmt::{FormatOptions, FormatValue, format_with};
use omacell_core::sheet::SheetVisibility;
use omacell_core::spill::SpillTable;
use omacell_core::storage::CellSlot;
use omacell_core::style::{Color as CellColor, Fill, HorizontalAlign, Underline};
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
use omacell_ui::{
    EditSurface, Palette, PanelState, StatusLine, UiSession, Viewport, conditional_format_ranges,
};
use ratatui::backend::TestBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::theme::{AnsiRoles, file_color};

/// Inputs for one virtualized frame.
pub struct FrameInput<'a> {
    /// Live workbook.
    pub wb: &'a Workbook,
    /// Recalculation spill regions.
    pub spill: &'a SpillTable,
    /// Worker-resolved conditional formats for this reader snapshot.
    pub conditional_formats: Option<&'a ConditionalFormatSnapshot>,
    /// WP-14 session.
    pub ui: &'a UiSession,
    /// Resolved theme name for the status line.
    pub theme_name: &'a str,
    /// File-origin RGB allowed.
    pub truecolor: bool,
    /// Unicode box drawing.
    pub unicode_borders: bool,
    /// Transient status message.
    pub message: Option<&'a str>,
    /// Highlighted command-palette row.
    pub palette_index: usize,
    /// Workbook has unsaved changes.
    pub dirty: bool,
    /// The command bus is occupied; this frame uses the last reader snapshot.
    pub busy: bool,
}

/// Exact terminal-cell geometry retained for mouse hit testing.
#[derive(Clone, Debug, Default)]
pub(crate) struct GridHitMap {
    columns: Vec<HitColumn>,
    rows: Vec<HitRow>,
}

impl GridHitMap {
    /// Map a terminal coordinate to a workbook row and column.
    pub(crate) fn hit(&self, x: u16, y: u16) -> Option<(u32, u16)> {
        let row = self
            .rows
            .iter()
            .find(|row| y >= row.start && y < row.end)?
            .index;
        let col = self
            .columns
            .iter()
            .find(|col| x >= col.start && x < col.end)?
            .index;
        Some((row, col))
    }

    pub(crate) fn conditional_format_ranges(
        &self,
        viewport: &Viewport,
    ) -> Vec<omacell_core::addr::RangeRef> {
        let rows = self.rows.iter().map(|row| row.index).collect::<Vec<_>>();
        let cols = self.columns.iter().map(|col| col.index).collect::<Vec<_>>();
        conditional_format_ranges(&rows, &cols, viewport.freeze)
    }
}

#[derive(Clone, Copy, Debug)]
struct HitColumn {
    index: u16,
    start: u16,
    end: u16,
    width: u16,
}

#[derive(Clone, Copy, Debug)]
struct HitRow {
    index: u32,
    start: u16,
    end: u16,
    height: u16,
}

/// Draw one frame. Only the visible window is visited.
pub fn draw(frame: &mut Frame<'_>, input: FrameInput<'_>) -> GridHitMap {
    let FrameInput {
        wb,
        spill,
        conditional_formats,
        ui,
        theme_name,
        truecolor,
        unicode_borders,
        message,
        palette_index,
        dirty,
        busy,
    } = input;
    let area = frame.area();
    let cfg = ui.config();
    let compact = u32::from(area.width).saturating_mul(8) < cfg.layout.compact_below_width;
    let show_tabs = cfg.appearance.show_sheet_tabs && !compact;
    let tabs_top = !cfg
        .appearance
        .sheet_tabs_position
        .eq_ignore_ascii_case("bottom");
    let show_formula = cfg.appearance.show_formula_bar;
    let show_status = cfg.appearance.show_status_line;
    let col_chars = col_width_chars(cfg.appearance.column_width);

    let mut chunks = Vec::new();
    if show_tabs && tabs_top {
        chunks.push(Constraint::Length(1));
    }
    if show_formula {
        let lines = if ui.formula_bar_expanded() && !compact {
            cfg.layout.formula_bar_lines.max(2)
        } else {
            1
        };
        chunks.push(Constraint::Length(lines as u16));
    }
    chunks.push(Constraint::Min(3));
    if show_tabs && !tabs_top {
        chunks.push(Constraint::Length(1));
    }
    if show_status {
        chunks.push(Constraint::Length(1));
    }
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(chunks)
        .split(area);

    let mut i = 0;
    if show_tabs && tabs_top {
        draw_tabs(frame, layout[i], wb, ui.selection().sheet);
        i += 1;
    }
    if show_formula {
        draw_formula_bar(frame, layout[i], wb, ui, truecolor);
        i += 1;
    }
    let grid = layout[i];
    size_viewport(ui, grid, col_chars);
    let mut hit_map = draw_grid(
        frame,
        grid,
        wb,
        spill,
        conditional_formats,
        ui,
        unicode_borders,
        col_chars,
        truecolor,
    );
    i += 1;
    if show_tabs && !tabs_top {
        draw_tabs(frame, layout[i], wb, ui.selection().sheet);
        i += 1;
    }
    if show_status {
        draw_status(
            frame,
            layout[i],
            ui,
            wb,
            theme_name,
            &cfg.layout.status_line,
            message,
            dirty,
            busy,
        );
    }

    let palette = ui.palette();
    if palette.open {
        draw_palette(frame, area, &palette, palette_index, unicode_borders);
        hit_map = GridHitMap::default();
    }
    let panel = ui.panel();
    if let Some(id) = &panel.visible {
        draw_panel(frame, area, &panel, id, ui, unicode_borders);
        hit_map = GridHitMap::default();
    }
    hit_map
}

fn col_width_chars(width: f64) -> u16 {
    if !width.is_finite() {
        return 8;
    }
    width.round().clamp(4.0, 24.0) as u16
}

fn size_viewport(ui: &UiSession, area: Rect, col_chars: u16) {
    let mut vp = ui.viewport();
    let header_h = 1u16;
    let header_w = row_header_width(&vp, area);
    let rows = area.height.saturating_sub(header_h).max(1);
    let chars = area.width.saturating_sub(header_w).max(1);
    vp.height_px = u32::from(rows).saturating_mul(DEFAULT_ROW_PX);
    vp.width_px = u32::from(chars)
        .saturating_mul(DEFAULT_COL_PX)
        .checked_div(u32::from(col_chars.max(1)))
        .unwrap_or(DEFAULT_COL_PX)
        .max(1);
    ui.set_viewport(vp);
}

fn row_header_width(vp: &Viewport, area: Rect) -> u16 {
    let last = vp
        .screen_rows()
        .2
        .saturating_add(vp.freeze.rows)
        .saturating_add(u32::from(area.height));
    let digits = last.saturating_add(1).to_string().len() as u16;
    digits.max(4)
}

fn draw_tabs(frame: &mut Frame<'_>, area: Rect, wb: &Workbook, active: SheetId) {
    let mut spans = Vec::new();
    for sheet in wb
        .sheets()
        .filter(|sheet| sheet.visibility == SheetVisibility::Visible || sheet.id == active)
    {
        if !spans.is_empty() {
            spans.push(Span::raw(" | "));
        }
        let name = terminal_text(&sheet.name);
        if sheet.id == active {
            spans.push(Span::styled(
                name,
                Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            ));
        } else {
            spans.push(Span::raw(name));
        }
    }
    if spans.is_empty() {
        spans.push(Span::raw("Sheet1"));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_formula_bar(
    frame: &mut Frame<'_>,
    area: Rect,
    wb: &Workbook,
    ui: &UiSession,
    truecolor: bool,
) {
    let edit = ui.edit();
    let sel = ui.selection();
    let addr = format!(
        "{}{}",
        col_to_letters(sel.cursor.col).unwrap_or_else(|_| "A".into()),
        sel.cursor.row + 1
    );
    let mut spans = vec![
        Span::styled(addr.clone(), Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::raw("fx "),
    ];
    if edit.is_idle() {
        let locale = configured_locale(&ui.config().locale.language);
        let (text, _) = cell_text(
            wb,
            sel.sheet,
            sel.cursor.row,
            sel.cursor.col,
            true,
            locale,
            None,
        );
        spans.push(Span::raw(terminal_text(&text)));
    } else {
        spans.extend(colorize_formula(
            &edit.buffer,
            &edit.reference_spans(),
            ui,
            truecolor,
        ));
        if let Some(ghost) = &edit.ghost {
            spans.push(Span::styled(
                terminal_text(ghost),
                Style::default().add_modifier(Modifier::DIM),
            ));
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).wrap(Wrap { trim: false }),
        area,
    );
    if !edit.is_idle() && area.width > 0 && area.height > 0 {
        let prefix = text_width(&format!("{addr}  fx "));
        let cursor = edit.cursor.min(edit.buffer.len());
        let cursor = floor_char_boundary(&edit.buffer, cursor);
        let offset = prefix.saturating_add(text_width(&terminal_text(&edit.buffer[..cursor])));
        let width = usize::from(area.width.max(1));
        let x = area
            .x
            .saturating_add(u16::try_from(offset % width).unwrap_or(u16::MAX))
            .min(area.right().saturating_sub(1));
        let y = area
            .y
            .saturating_add(u16::try_from(offset / width).unwrap_or(u16::MAX))
            .min(area.bottom().saturating_sub(1));
        frame.set_cursor_position((x, y));
    }
}

fn colorize_formula(
    buffer: &str,
    spans: &[(usize, usize, usize)],
    ui: &UiSession,
    truecolor: bool,
) -> Vec<Span<'static>> {
    if spans.is_empty() {
        return vec![Span::raw(buffer.to_string())];
    }
    let mut ordered = spans.to_vec();
    ordered.sort_by_key(|s| s.0);
    let mut out = Vec::new();
    let mut cur = 0usize;
    for (start, end, idx) in ordered {
        if start < cur || end > buffer.len() || start >= end {
            continue;
        }
        if start > cur {
            out.push(Span::raw(terminal_text(&buffer[cur..start])));
        }
        let hex = ui.reference_color(idx);
        out.push(Span::styled(
            terminal_text(&buffer[start..end]),
            Style::default().fg(file_color(&hex, truecolor)),
        ));
        cur = end;
    }
    if cur < buffer.len() {
        out.push(Span::raw(terminal_text(&buffer[cur..])));
    }
    if out.is_empty() {
        out.push(Span::raw(terminal_text(buffer)));
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn draw_grid(
    frame: &mut Frame<'_>,
    area: Rect,
    wb: &Workbook,
    spill: &SpillTable,
    conditional_formats: Option<&ConditionalFormatSnapshot>,
    ui: &UiSession,
    unicode: bool,
    col_chars: u16,
    truecolor: bool,
) -> GridHitMap {
    let ansi = AnsiRoles::default();
    let vp = ui.viewport();
    let sel = ui.selection();
    let sheet = sel.sheet;
    let show_formulas = ui.show_formulas();
    let cfg = ui.config();
    let locale = configured_locale(&cfg.locale.language);
    let grid_lines =
        cfg.appearance.grid_lines && wb.sheet(sheet).is_none_or(|sheet| sheet.view.gridlines);
    let header_h = 1u16;
    let header_w = row_header_width(&vp, area);
    let (first_row, _, _) = vp.screen_rows();
    let first_col = vp.first_col.max(vp.freeze.cols);
    let vbar = if !grid_lines {
        ""
    } else if unicode {
        "│"
    } else {
        "|"
    };
    let hbar = if unicode { '─' } else { '-' };

    let columns = visible_columns(&vp, first_col, area, header_w, col_chars, grid_lines);
    let rows = visible_rows(&vp, first_row, area, header_h);
    let mut lines: Vec<Line<'_>> = Vec::new();
    let mut header = vec![Span::styled(
        format!("{:width$}", "", width = usize::from(header_w)),
        Style::default().fg(ansi.header),
    )];
    for (index, col) in columns.iter().enumerate() {
        if grid_lines {
            let freeze_edge = vp.freeze.cols > 0
                && col.index >= vp.freeze.cols
                && index > 0
                && columns[index - 1].index < vp.freeze.cols;
            header.push(Span::styled(
                if freeze_edge && unicode {
                    "┃".to_string()
                } else {
                    vbar.to_string()
                },
                Style::default().fg(ansi.grid),
            ));
        }
        let label = col_to_letters(col.index).unwrap_or_else(|_| "?".into());
        header.push(Span::styled(
            pad(&label, usize::from(col.width), Align::Center),
            Style::default()
                .fg(ansi.header)
                .add_modifier(Modifier::BOLD),
        ));
    }
    lines.push(Line::from(header));

    let selected_areas = sel
        .areas
        .iter()
        .map(|area| area.normalized())
        .collect::<Vec<_>>();
    let review = ui.changeset_review();
    let review_sheet = wb.sheet(sheet).map(|sheet| sheet.name.as_str());
    let cursor_style = cfg.appearance.cursor_style.as_str();
    let selection_style = cfg.appearance.selection_style.as_str();
    let zebra = cfg.appearance.zebra_rows;

    let edit = ui.edit();
    for (vis_i, row) in rows.iter().enumerate() {
        for subline in 0..row.height {
            let freeze_edge = vp.freeze.rows > 0
                && row.index < vp.freeze.rows
                && rows
                    .get(vis_i.saturating_add(1))
                    .is_none_or(|next| next.index >= vp.freeze.rows)
                && subline.saturating_add(1) == row.height;
            let mut header_style = Style::default().fg(ansi.header);
            if freeze_edge {
                header_style = header_style.add_modifier(Modifier::UNDERLINED);
            }
            let header_text = if subline == 0 {
                format!("{:>width$}", row.index + 1, width = usize::from(header_w))
            } else {
                " ".repeat(usize::from(header_w))
            };
            let mut spans = vec![Span::styled(header_text, header_style)];
            let mut col_i = 0usize;
            while col_i < columns.len() {
                let col = columns[col_i];
                if grid_lines {
                    let freeze_col_edge = vp.freeze.cols > 0
                        && col.index >= vp.freeze.cols
                        && col_i > 0
                        && columns[col_i - 1].index < vp.freeze.cols;
                    let bar = if freeze_col_edge && unicode {
                        "┃"
                    } else {
                        vbar
                    };
                    spans.push(Span::styled(
                        bar.to_string(),
                        Style::default().fg(ansi.grid),
                    ));
                }
                let slot = wb.get(sheet, row.index, col.index).ok().flatten();
                let overlay =
                    conditional_formats.and_then(|resolved| resolved.get(row.index, col.index));
                let editing_here = subline == 0
                    && edit.surface == EditSurface::InCell
                    && edit.origin.is_some_and(|origin| {
                        origin.sheet.unwrap_or(sheet) == sheet
                            && origin.row == row.index
                            && origin.col == col.index
                    });
                let (mut text, align) = if editing_here {
                    (
                        format!(
                            "{}{}",
                            edit.buffer,
                            edit.ghost.as_deref().unwrap_or_default()
                        ),
                        Align::Left,
                    )
                } else if subline == 0 {
                    cell_text(
                        wb,
                        sheet,
                        row.index,
                        col.index,
                        show_formulas,
                        locale,
                        Some(usize::from(col.width)),
                    )
                } else {
                    (String::new(), Align::Left)
                };
                if subline == 0
                    && let Some(prefix) = overlay
                        .and_then(|overlay| overlay.visual)
                        .and_then(conditional_visual_prefix)
                {
                    text = format!("{prefix} {text}");
                }
                let (overflow, width) = if subline == 0 {
                    overflow_cols(
                        wb,
                        sheet,
                        row.index,
                        &columns[col_i + 1..],
                        text_width(&terminal_text(&text)),
                        usize::from(col.width),
                        align,
                        grid_lines,
                    )
                } else {
                    (0, usize::from(col.width))
                };
                let in_sel = selected_areas.iter().any(|&(r0, c0, r1, c1)| {
                    row.index >= r0 && row.index <= r1 && col.index >= c0 && col.index <= c1
                });
                let is_cursor = sel.cursor.row == row.index && sel.cursor.col == col.index;
                let mut style = cell_style(
                    wb,
                    slot,
                    overlay,
                    ansi,
                    zebra && vis_i % 2 == 1,
                    in_sel,
                    is_cursor,
                    text.starts_with('#'),
                    cursor_style,
                    selection_style,
                    truecolor,
                );
                if slot.is_some_and(|slot| slot.flags.stale()) {
                    style = style.add_modifier(Modifier::DIM);
                }
                if let Some(mark) = review.as_ref().and_then(|review| {
                    review_sheet.and_then(|name| review.cell_mark(name, row.index, col.index))
                }) {
                    style = style.bg(if mark.accepted {
                        ratatui::style::Color::Indexed(2)
                    } else {
                        ratatui::style::Color::Indexed(1)
                    });
                    if mark.selected {
                        style = style.add_modifier(Modifier::BOLD | Modifier::REVERSED);
                    }
                }
                if let Some(region) = spill.region_at(sheet, row.index, col.index) {
                    let on_edge = row.index == region.origin.row
                        || col.index == region.origin.col
                        || row.index
                            == region
                                .origin
                                .row
                                .saturating_add(region.rows.saturating_sub(1))
                        || col.index
                            == region
                                .origin
                                .col
                                .saturating_add((region.cols.saturating_sub(1)) as u16);
                    if on_edge {
                        style = style.add_modifier(Modifier::UNDERLINED);
                    }
                }
                if freeze_edge {
                    style = style.add_modifier(Modifier::UNDERLINED);
                }
                let display = if slot.is_some_and(|slot| slot.flags.stale()) && subline == 0 {
                    hatch(&pad(&text, width, align), hbar)
                } else {
                    pad(&text, width, align)
                };
                spans.push(Span::styled(display, style));
                col_i += 1 + overflow;
            }
            lines.push(Line::from(spans));
        }
    }
    frame.render_widget(Paragraph::new(lines), area);
    GridHitMap { columns, rows }
}

#[allow(clippy::too_many_arguments)]
fn overflow_cols(
    wb: &Workbook,
    sheet: SheetId,
    row: u32,
    rest: &[HitColumn],
    text_len: usize,
    col_chars: usize,
    align: Align,
    grid_lines: bool,
) -> (usize, usize) {
    if !matches!(align, Align::Left) || text_len <= col_chars {
        return (0, col_chars);
    }
    let mut extra = 0usize;
    let mut covered = col_chars;
    for next in rest {
        if covered >= text_len {
            break;
        }
        if !cell_empty(wb, sheet, row, next.index) {
            break;
        }
        extra += 1;
        covered += usize::from(next.width) + usize::from(grid_lines);
    }
    (extra, covered)
}

fn cell_empty(wb: &Workbook, sheet: SheetId, row: u32, col: u16) -> bool {
    match wb.get(sheet, row, col) {
        Ok(Some(slot)) => matches!(slot.value, Value::Empty) && slot.formula.is_none(),
        _ => true,
    }
}

fn visible_columns(
    vp: &Viewport,
    first: u16,
    area: Rect,
    header_width: u16,
    base_chars: u16,
    grid_lines: bool,
) -> Vec<HitColumn> {
    let mut columns = Vec::new();
    let mut x = area.x.saturating_add(header_width);
    append_columns(
        &mut columns,
        vp,
        &mut x,
        area.right(),
        0,
        u32::from(vp.freeze.cols),
        base_chars,
        grid_lines,
    );
    append_columns(
        &mut columns,
        vp,
        &mut x,
        area.right(),
        u32::from(first.max(vp.freeze.cols)),
        u32::from(omacell_core::limits::MAX_COLS),
        base_chars,
        grid_lines,
    );
    columns
}

#[allow(clippy::too_many_arguments)]
fn append_columns(
    output: &mut Vec<HitColumn>,
    vp: &Viewport,
    x: &mut u16,
    right: u16,
    mut candidate: u32,
    limit: u32,
    base_chars: u16,
    grid_lines: bool,
) {
    while *x < right && candidate < limit {
        let Some(index) = next_visible(&vp.cols, candidate, limit) else {
            break;
        };
        let separator = u16::from(grid_lines);
        let start = x.saturating_add(separator);
        if start >= right {
            break;
        }
        let natural = column_width(vp, index as u16, base_chars);
        let width = natural.min(right.saturating_sub(start));
        if width == 0 {
            break;
        }
        let end = start.saturating_add(width);
        output.push(HitColumn {
            index: index as u16,
            start,
            end,
            width,
        });
        *x = end;
        candidate = index.saturating_add(1);
    }
}

fn visible_rows(vp: &Viewport, first: u32, area: Rect, header_height: u16) -> Vec<HitRow> {
    let mut rows = Vec::new();
    let mut y = area.y.saturating_add(header_height);
    append_rows(
        &mut rows,
        vp,
        &mut y,
        area.bottom(),
        0,
        vp.freeze.rows.min(omacell_core::limits::MAX_ROWS),
    );
    append_rows(
        &mut rows,
        vp,
        &mut y,
        area.bottom(),
        first.max(vp.freeze.rows),
        omacell_core::limits::MAX_ROWS,
    );
    rows
}

fn append_rows(
    output: &mut Vec<HitRow>,
    vp: &Viewport,
    y: &mut u16,
    bottom: u16,
    mut candidate: u32,
    limit: u32,
) {
    while *y < bottom && candidate < limit {
        let Some(index) = next_visible(&vp.rows, candidate, limit) else {
            break;
        };
        let natural = row_height(vp, index);
        let height = natural.min(bottom.saturating_sub(*y));
        if height == 0 {
            break;
        }
        let end = y.saturating_add(height);
        output.push(HitRow {
            index,
            start: *y,
            end,
            height,
        });
        *y = end;
        candidate = index.saturating_add(1);
    }
}

fn next_visible(axis: &AxisGeometry, start: u32, limit: u32) -> Option<u32> {
    if start >= limit || start >= axis.len() {
        return None;
    }
    let pixel = axis.index_to_pixel(start);
    if pixel >= axis.total_px() {
        return None;
    }
    let index = axis.pixel_to_index(pixel);
    (index >= start && index < limit && axis.size(index).ok()? > 0).then_some(index)
}

fn column_width(vp: &Viewport, col: u16, base_chars: u16) -> u16 {
    let pixels = vp.col_px(col);
    let scaled = f64::from(base_chars) * f64::from(pixels) / f64::from(DEFAULT_COL_PX);
    scaled.round().clamp(1.0, 192.0) as u16
}

fn row_height(vp: &Viewport, row: u32) -> u16 {
    let pixels = vp.row_px(row);
    let lines = pixels.saturating_add(DEFAULT_ROW_PX - 1) / DEFAULT_ROW_PX;
    u16::try_from(lines.clamp(1, 32)).unwrap_or(32)
}

#[allow(clippy::too_many_arguments)]
fn cell_style(
    wb: &Workbook,
    slot: Option<&CellSlot>,
    overlay: Option<CfOverlay>,
    ansi: AnsiRoles,
    zebra: bool,
    selected: bool,
    cursor: bool,
    error: bool,
    cursor_style: &str,
    selection_style: &str,
    truecolor: bool,
) -> Style {
    let mut style = Style::default().fg(ansi.fg);
    if zebra {
        style = style.bg(ansi.zebra);
    }
    if let Some(cell_style) = slot.and_then(|slot| wb.intern().styles.get(slot.style)) {
        if cell_style.font.bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        if cell_style.font.italic {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if cell_style.font.underline != Underline::None {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        if cell_style.font.strike {
            style = style.add_modifier(Modifier::CROSSED_OUT);
        }
        if let Some(color) = workbook_color(cell_style.font.color, truecolor) {
            style = style.fg(color);
        }
        let fill = match cell_style.fill {
            Fill::Solid { fg } | Fill::Pattern { fg, .. } => workbook_color(fg, truecolor),
            Fill::None | Fill::Gradient(_) => None,
        };
        if let Some(color) = fill {
            style = style.bg(color);
        }
    }
    if let Some(overlay) = overlay {
        if let Some(color) = overlay
            .font
            .and_then(|color| workbook_color(color, truecolor))
        {
            style = style.fg(color);
        }
        if let Some(color) = overlay
            .fill
            .and_then(|color| workbook_color(color, truecolor))
        {
            style = style.bg(color);
        }
    }
    if error {
        style = style.fg(ansi.error);
    }
    if selected {
        style = match selection_style {
            "outline" => style.add_modifier(Modifier::BOLD),
            _ => style.bg(ansi.selection),
        };
    }
    if cursor {
        style = match cursor_style {
            "underline" => style.fg(ansi.cursor).add_modifier(Modifier::UNDERLINED),
            "outline" => style.fg(ansi.cursor).add_modifier(Modifier::BOLD),
            _ => style.fg(ansi.cursor).add_modifier(Modifier::REVERSED),
        };
    }
    style
}

fn workbook_color(color: CellColor, truecolor: bool) -> Option<ratatui::style::Color> {
    match color {
        CellColor::Auto | CellColor::Theme { .. } => None,
        CellColor::Indexed { index } => Some(ratatui::style::Color::Indexed(index)),
        CellColor::Rgb { argb } if truecolor => Some(ratatui::style::Color::Rgb(
            ((argb >> 16) & 0xff) as u8,
            ((argb >> 8) & 0xff) as u8,
            (argb & 0xff) as u8,
        )),
        CellColor::Rgb { .. } => None,
    }
}

fn conditional_visual_prefix(visual: CfVisual) -> Option<&'static str> {
    match visual {
        CfVisual::Icon { icons, index } => {
            let glyphs: &[&str] = match icons {
                3 => &["▼", "◆", "▲"],
                4 => &["▼", "◀", "▶", "▲"],
                _ => &["▼", "◢", "◆", "◤", "▲"],
            };
            glyphs.get(usize::from(index)).copied()
        }
        CfVisual::DataBar { fraction, axis, .. } => {
            if !fraction.is_finite() || !axis.is_finite() {
                return None;
            }
            let magnitude = (fraction - axis).abs().clamp(0.0, 1.0);
            let index = (magnitude * 7.0).round() as usize;
            ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"].get(index).copied()
        }
    }
}

fn hatch(text: &str, mark: char) -> String {
    text.chars()
        .enumerate()
        .map(|(i, c)| if i % 2 == 1 { mark } else { c })
        .collect()
}

#[derive(Clone, Copy)]
enum Align {
    Left,
    Right,
    Center,
}

fn pad(text: &str, width: usize, align: Align) -> String {
    let safe = terminal_text(text);
    let (text, used) = truncate_to_width(&safe, width);
    let remaining = width.saturating_sub(used);
    let (left, right) = match align {
        Align::Left => (0, remaining),
        Align::Right => (remaining, 0),
        Align::Center => (remaining / 2, remaining - remaining / 2),
    };
    format!("{}{}{}", " ".repeat(left), text, " ".repeat(right))
}

fn terminal_text(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() || is_bidi_control(character) {
                '�'
            } else {
                character
            }
        })
        .collect()
}

fn terminal_multiline(text: &str) -> String {
    text.split('\n')
        .map(terminal_text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

fn truncate_to_width(text: &str, width: usize) -> (String, usize) {
    let mut output = String::new();
    let mut used = 0usize;
    for character in text.chars() {
        let character_width = text_width(&character.to_string());
        if used.saturating_add(character_width) > width {
            break;
        }
        output.push(character);
        used = used.saturating_add(character_width);
    }
    (output, used)
}

fn text_width(text: &str) -> usize {
    Span::raw(text).width()
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn configured_locale(language: &str) -> LocaleId {
    if !language.eq_ignore_ascii_case("system") {
        return LocaleId::parse_tag(language).unwrap_or(LocaleId::EN_US);
    }
    ["LC_ALL", "LC_NUMERIC", "LANG"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .map(|value| {
            value
                .split(['.', '@'])
                .next()
                .unwrap_or(value.as_str())
                .replace('_', "-")
        })
        .find_map(|tag| LocaleId::parse_tag(&tag))
        .unwrap_or(LocaleId::EN_US)
}

fn cell_text(
    wb: &Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
    formulas: bool,
    locale: LocaleId,
    width: Option<usize>,
) -> (String, Align) {
    if let Some(spark) = wb.sheet(sheet).and_then(|s| {
        s.sparklines
            .iter()
            .find(|sp| sp.row == row && sp.col == col)
    }) && let Ok(sampled) = omacell_core::chart::sample(
        wb,
        &omacell_core::chart::Chart {
            id: omacell_core::chart::ChartId::new(0),
            kind: omacell_core::chart::ChartKind::Line,
            title: None,
            categories: None,
            series: vec![omacell_core::chart::Series {
                name: String::new(),
                values: spark.data,
                x: None,
                size: None,
                color: None,
                secondary_axis: false,
                trendline: None,
            }],
            category_axis: omacell_core::chart::Axis::default(),
            value_axis: omacell_core::chart::Axis::default(),
            secondary_axis: None,
            legend: omacell_core::chart::LegendPos::None,
            data_labels: false,
            anchor: omacell_core::chart::ChartAnchor::default(),
            sheet,
        },
    ) && let Some(series) = sampled.series.first()
    {
        return (spark_glyphs(&series.y, spark.kind), Align::Left);
    }
    let Ok(Some(slot)) = wb.get(sheet, row, col) else {
        return (String::new(), Align::Left);
    };
    if formulas && let Some(fid) = slot.formula {
        let src = wb.intern().formulas.get(fid).unwrap_or("");
        return (src.to_string(), Align::Left);
    }
    let (text, value_align) = match slot.value {
        Value::Empty => (String::new(), Align::Left),
        Value::Number(n) => {
            let code = wb
                .intern()
                .styles
                .get(slot.style)
                .map(|s| s.num_fmt)
                .and_then(|id| wb.num_fmt_code(id))
                .unwrap_or_else(|| "General".into());
            let mut options = FormatOptions::new(locale);
            options.date_system = wb.settings().date_system;
            options.width = width;
            let formatted = format_with(FormatValue::Number(n), code.as_ref(), &options);
            (formatted.text, Align::Right)
        }
        Value::Bool(true) => ("TRUE".into(), Align::Left),
        Value::Bool(false) => ("FALSE".into(), Align::Left),
        Value::Text(id) => (
            wb.intern().strings.get(id).unwrap_or("").to_string(),
            Align::Left,
        ),
        Value::Error(kind) => (kind.as_str().to_string(), Align::Left),
        Value::Array(_) => (String::new(), Align::Left),
    };
    (text, style_align(wb, slot, value_align))
}

fn spark_glyphs(values: &[f64], kind: omacell_core::chart::SparklineKind) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    match kind {
        omacell_core::chart::SparklineKind::WinLoss => values
            .iter()
            .map(|v| {
                if *v > 0.0 {
                    '▲'
                } else if *v < 0.0 {
                    '▼'
                } else {
                    '•'
                }
            })
            .collect(),
        _ => {
            let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
            let min = finite.iter().copied().fold(f64::INFINITY, f64::min);
            let max = finite.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let span = (max - min).max(1e-9);
            values
                .iter()
                .map(|v| {
                    if !v.is_finite() {
                        ' '
                    } else {
                        let t = ((*v - min) / span * 7.0).round() as usize;
                        BARS[t.min(7)]
                    }
                })
                .collect()
        }
    }
}

fn style_align(wb: &Workbook, slot: &CellSlot, fallback: Align) -> Align {
    let Some(style) = wb.intern().styles.get(slot.style) else {
        return fallback;
    };
    match style.alignment.horizontal {
        HorizontalAlign::Left | HorizontalAlign::Fill | HorizontalAlign::Justify => Align::Left,
        HorizontalAlign::Right => Align::Right,
        HorizontalAlign::Center
        | HorizontalAlign::CenterContinuous
        | HorizontalAlign::Distributed => Align::Center,
        HorizontalAlign::General => fallback,
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_status(
    frame: &mut Frame<'_>,
    area: Rect,
    ui: &UiSession,
    wb: &Workbook,
    theme_name: &str,
    ids: &[String],
    message: Option<&str>,
    dirty: bool,
    busy: bool,
) {
    let sel = ui.selection();
    let cell = format!(
        "{}{}",
        col_to_letters(sel.cursor.col).unwrap_or_else(|_| "A".into()),
        sel.cursor.row + 1
    );
    let stats = format!("Cnt {}", sel.cell_count());
    let calc = match wb.settings().calc_mode {
        omacell_core::workbook::CalcMode::Manual => "Manual",
        omacell_core::workbook::CalcMode::AutomaticExceptTables => "Auto*",
        omacell_core::workbook::CalcMode::Automatic => "Auto",
    };
    let mut line = StatusLine::default();
    line.refresh(ids, ui.mode().label(), &cell, &stats, calc, theme_name);
    let cfg = ui.config();
    let local = cfg
        .ai
        .models
        .default
        .split(':')
        .next()
        .and_then(|name| cfg.ai.providers.get(name).map(|p| p.local))
        .unwrap_or(false);
    line.set_ai(Some(omacell_ui::ai_status_text(
        cfg.ai.enabled,
        &cfg.ai.models.default,
        local,
        &cfg.ai.privacy.send,
    )));
    line.set_offer(omacell_ui::diagnose_offer(
        wb,
        ui.config().ai.agent.diagnose_offers,
        ui.agent_visible(),
    ));
    let zoom = format!("{}%", (ui.viewport().zoom * 100.0).round());
    let dirty = if dirty { "*" } else { "" };
    for seg in &mut line.segments {
        match seg.id.as_str() {
            "zoom" => seg.text = zoom.clone(),
            "dirty" => seg.text = dirty.to_string(),
            _ => {}
        }
    }
    let mut text = line
        .segments
        .iter()
        .map(|s| s.text.as_str())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("  ");
    if let Some(msg) = message {
        text.push_str("  ");
        text.push_str(&terminal_text(msg));
    }
    if busy {
        text.push_str("  working…");
    }
    frame.render_widget(Paragraph::new(terminal_text(&text)), area);
}

fn draw_palette(
    frame: &mut Frame<'_>,
    area: Rect,
    palette: &Palette,
    selected: usize,
    unicode: bool,
) {
    let popup = centered(area, 60, 12);
    frame.render_widget(Clear, popup);
    let first = selected.saturating_sub(7);
    let items: Vec<ListItem<'_>> = palette
        .hits
        .iter()
        .skip(first)
        .take(8)
        .enumerate()
        .map(|(index, hit)| {
            let item = ListItem::new(terminal_text(&format!("{}  {}", hit.id, hit.doc)));
            if first.saturating_add(index) == selected {
                item.style(Style::default().add_modifier(Modifier::REVERSED))
            } else {
                item
            }
        })
        .collect();
    let title = if let Some(prompt) = &palette.prompt {
        terminal_text(&format!("{prompt}: {}", palette.query))
    } else {
        terminal_text(&format!("palette: {}", palette.query))
    };
    if let Some(preview) = &palette.preview {
        frame.render_widget(
            Paragraph::new(terminal_text(preview))
                .wrap(Wrap { trim: false })
                .block(chrome_block(title, unicode)),
            popup,
        );
    } else {
        frame.render_widget(List::new(items).block(chrome_block(title, unicode)), popup);
    }
}

fn draw_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    panel: &PanelState,
    id: &str,
    ui: &UiSession,
    unicode: bool,
) {
    let width = (panel.width / 8).clamp(20, 40) as u16;
    let popup = match panel.side.as_str() {
        "left" => Rect {
            x: area.x,
            y: area.y,
            width: width.min(area.width),
            height: area.height,
        },
        "bottom" => Rect {
            x: area.x,
            y: area.bottom().saturating_sub(8),
            width: area.width,
            height: 8.min(area.height),
        },
        _ => Rect {
            x: area.right().saturating_sub(width),
            y: area.y,
            width: width.min(area.width),
            height: area.height,
        },
    };
    frame.render_widget(Clear, popup);
    let body = panel.body.clone().unwrap_or_else(|| match id {
        "find" => {
            let f = ui.find_replace();
            format!("find: {}\nreplace: {}", f.find, f.replace)
        }
        "goto" => format!("goto: {}", ui.goto().target),
        "keys" => "F1 keys overlay\nEsc closes panels\nCtrl+Q quits".into(),
        "changeset" => ui
            .changeset_review()
            .map_or_else(|| "no proposed changesets".into(), |review| review.body()),
        "agent" => ui.agent_panel().body(),
        "formula" => {
            let mut body = ui
                .formula_assist()
                .map_or_else(|| "no formula-assist result".into(), |assist| assist.body());
            if let Some(review) = ui.changeset_review() {
                body.push_str("\n\n");
                body.push_str(&review.body());
            }
            body
        }
        "comments" => "comments (WP-19)".into(),
        "format" => "format panel (WP-18)".into(),
        "sort" | "filter" => format!("{id} panel (WP-17)"),
        other => format!("{other} panel"),
    });
    frame.render_widget(
        Paragraph::new(terminal_multiline(&body)).block(
            chrome_block(terminal_text(id), unicode)
                .border_style(Style::default().fg(AnsiRoles::default().cursor)),
        ),
        popup,
    );
}

fn chrome_block(title: impl Into<String>, unicode: bool) -> Block<'static> {
    let block = Block::default().title(title.into()).borders(Borders::ALL);
    if unicode {
        block
    } else {
        block.border_set(ratatui::symbols::border::PLAIN)
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

/// Public so benches can size a viewport without drawing.
pub fn prepare_viewport(ui: &UiSession, width: u16, height: u16) {
    size_viewport(
        ui,
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        },
        8,
    );
}

/// Dump a [`TestBackend`] buffer as plain text (snapshots).
#[must_use]
pub fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    let buf = terminal.backend().buffer();
    let area = buf.area();
    let mut out = String::with_capacity(usize::from(area.width) * usize::from(area.height));
    for y in area.y..area.y.saturating_add(area.height) {
        let start = out.len();
        for x in area.x..area.x.saturating_add(area.width) {
            out.push_str(buf[(x, y)].symbol());
        }
        out.truncate(out[start..].trim_end_matches(' ').len() + start);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::conditional_visual_prefix;
    use omacell_core::condfmt::CfVisual;
    use omacell_core::style::Color;

    #[test]
    fn conditional_visuals_have_bounded_terminal_glyphs() {
        assert_eq!(
            conditional_visual_prefix(CfVisual::Icon { icons: 3, index: 2 }),
            Some("▲")
        );
        assert_eq!(
            conditional_visual_prefix(CfVisual::DataBar {
                color: Color::Auto,
                gradient: false,
                fraction: 1.0,
                axis: 0.0,
            }),
            Some("█")
        );
        assert_eq!(
            conditional_visual_prefix(CfVisual::DataBar {
                color: Color::Auto,
                gradient: false,
                fraction: f64::NAN,
                axis: 0.0,
            }),
            None
        );
    }
}
