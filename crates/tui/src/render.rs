//! Virtualized grid, formula bar, tabs, status, palette, and panels.

use omacell_core::addr::{SheetId, col_to_letters};
use omacell_core::geometry::{DEFAULT_COL_PX, DEFAULT_ROW_PX};
use omacell_core::locale::LocaleId;
use omacell_core::numfmt::{FormatValue, format};
use omacell_core::recalc::RecalcEngine;
use omacell_core::spill::SpillTable;
use omacell_core::storage::CellSlot;
use omacell_core::style::HorizontalAlign;
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
use omacell_ui::{Palette, PanelState, StatusLine, UiSession, Viewport};
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
    /// Recalc engine (spill outlines).
    pub engine: &'a RecalcEngine,
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
}

/// Draw one frame. Only the visible window is visited.
pub fn draw(frame: &mut Frame<'_>, input: FrameInput<'_>) {
    let FrameInput {
        wb,
        engine,
        ui,
        theme_name,
        truecolor,
        unicode_borders,
        message,
    } = input;
    let area = frame.area();
    let cfg = ui.config();
    let compact = u32::from(area.width).saturating_mul(8) < cfg.layout.compact_below_width;
    let show_tabs = cfg.appearance.show_sheet_tabs && !compact;
    let show_formula = cfg.appearance.show_formula_bar;
    let show_status = cfg.appearance.show_status_line;
    let col_chars = col_width_chars(cfg.appearance.column_width);

    let mut chunks = Vec::new();
    if show_tabs {
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
    if show_status {
        chunks.push(Constraint::Length(1));
    }
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(chunks)
        .split(area);

    let mut i = 0;
    if show_tabs {
        draw_tabs(frame, layout[i], wb, ui.selection().sheet);
        i += 1;
    }
    if show_formula {
        draw_formula_bar(frame, layout[i], wb, ui, truecolor);
        i += 1;
    }
    let grid = layout[i];
    size_viewport(ui, grid, col_chars);
    draw_grid(
        frame,
        grid,
        wb,
        engine.spill(),
        ui,
        unicode_borders,
        col_chars,
    );
    if show_status {
        i += 1;
        draw_status(
            frame,
            layout[i],
            ui,
            wb,
            theme_name,
            &cfg.layout.status_line,
            message,
        );
    }

    let palette = ui.palette();
    if palette.open {
        draw_palette(frame, area, &palette, unicode_borders);
    }
    let panel = ui.panel();
    if let Some(id) = &panel.visible {
        draw_panel(frame, area, &panel, id, ui, unicode_borders);
    }
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
    let cell_w = col_chars.saturating_add(1).max(1);
    let cols = area.width.saturating_sub(header_w) / cell_w;
    vp.height_px = u32::from(rows) * DEFAULT_ROW_PX;
    vp.width_px = u32::from(cols.max(1)) * DEFAULT_COL_PX;
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
    for (i, sheet) in wb.sheets().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" | "));
        }
        let name = sheet.name.as_str();
        if sheet.id == active {
            spans.push(Span::styled(
                name.to_string(),
                Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            ));
        } else {
            spans.push(Span::raw(name.to_string()));
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
        Span::styled(addr, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::raw("fx "),
    ];
    if edit.is_idle() {
        let (text, _) = cell_text(wb, sel.sheet, sel.cursor.row, sel.cursor.col, true);
        spans.push(Span::raw(text));
    } else {
        spans.extend(colorize_formula(
            &edit.buffer,
            &edit.reference_spans(),
            ui,
            truecolor,
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).wrap(Wrap { trim: false }),
        area,
    );
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
            out.push(Span::raw(buffer[cur..start].to_string()));
        }
        let hex = ui.reference_color(idx);
        out.push(Span::styled(
            buffer[start..end].to_string(),
            Style::default().fg(file_color(&hex, truecolor)),
        ));
        cur = end;
    }
    if cur < buffer.len() {
        out.push(Span::raw(buffer[cur..].to_string()));
    }
    if out.is_empty() {
        out.push(Span::raw(buffer.to_string()));
    }
    out
}

fn draw_grid(
    frame: &mut Frame<'_>,
    area: Rect,
    wb: &Workbook,
    spill: &SpillTable,
    ui: &UiSession,
    unicode: bool,
    col_chars: u16,
) {
    let ansi = AnsiRoles::default();
    let vp = ui.viewport();
    let sel = ui.selection();
    let sheet = sel.sheet;
    let show_formulas = ui.show_formulas();
    let cfg = ui.config();
    let grid_lines = cfg.appearance.grid_lines;
    let header_h = 1u16;
    let header_w = row_header_width(&vp, area);
    let cell_w = col_chars.saturating_add(u16::from(grid_lines));
    let rows_fit = area.height.saturating_sub(header_h);
    let cols_fit = area.width.saturating_sub(header_w) / cell_w.max(1);
    let (first_row, _, _) = vp.screen_rows();
    let first_col = vp.first_col.max(vp.freeze.cols);
    let last_col = first_col.saturating_add(cols_fit.max(1).saturating_sub(1));
    let vbar = if !grid_lines {
        ""
    } else if unicode {
        "│"
    } else {
        "|"
    };
    let hbar = if unicode { '─' } else { '-' };

    let cols = visible_cols(&vp, first_col, last_col);
    let mut lines: Vec<Line<'_>> = Vec::new();
    let mut header = vec![Span::styled(
        format!("{:width$}", "", width = usize::from(header_w)),
        Style::default().fg(ansi.header),
    )];
    for col in &cols {
        if grid_lines {
            header.push(Span::styled(
                vbar.to_string(),
                Style::default().fg(ansi.grid),
            ));
        }
        let label = col_to_letters(*col).unwrap_or_else(|_| "?".into());
        header.push(Span::styled(
            pad(&label, usize::from(col_chars), Align::Center),
            Style::default()
                .fg(ansi.header)
                .add_modifier(Modifier::BOLD),
        ));
    }
    lines.push(Line::from(header));

    let mut row_list: Vec<u32> = (0..vp.freeze.rows).collect();
    let mut row = first_row;
    while row_list.len() < usize::from(rows_fit) && row < omacell_core::limits::MAX_ROWS {
        if !row_list.contains(&row) && vp.row_px(row) != 0 {
            row_list.push(row);
        }
        row = row.saturating_add(1);
        if row <= first_row {
            break;
        }
    }

    let active = sel.active();
    let (r0, c0, r1, c1) = active.normalized();
    let cursor_style = cfg.appearance.cursor_style.as_str();
    let selection_style = cfg.appearance.selection_style.as_str();
    let zebra = cfg.appearance.zebra_rows;

    for (vis_i, row) in row_list.into_iter().enumerate() {
        let mut spans = vec![Span::styled(
            format!("{:>width$}", row + 1, width = usize::from(header_w)),
            Style::default().fg(ansi.header),
        )];
        let mut col_i = 0usize;
        while col_i < cols.len() {
            let col = cols[col_i];
            if grid_lines {
                let freeze_edge = vp.freeze.cols > 0 && col == vp.freeze.cols;
                let bar = if freeze_edge && unicode { "┃" } else { vbar };
                spans.push(Span::styled(
                    bar.to_string(),
                    Style::default().fg(ansi.grid),
                ));
            }
            let (text, align) = cell_text(wb, sheet, row, col, show_formulas);
            let slot = wb.get(sheet, row, col).ok().flatten();
            let overflow = overflow_cols(
                wb,
                sheet,
                row,
                &cols[col_i + 1..],
                text.chars().count(),
                usize::from(col_chars),
                align,
            );
            let width = usize::from(col_chars) + overflow * usize::from(cell_w);
            let mut style = Style::default().fg(ansi.fg);
            if zebra && vis_i % 2 == 1 {
                style = style.bg(ansi.zebra);
            }
            let in_sel = row >= r0 && row <= r1 && col >= c0 && col <= c1;
            let is_cursor = sel.cursor.row == row && sel.cursor.col == col;
            if in_sel {
                style = match selection_style {
                    "outline" => style.add_modifier(Modifier::BOLD),
                    _ => style.bg(ansi.selection),
                };
            }
            if is_cursor {
                style = match cursor_style {
                    "underline" => style.fg(ansi.cursor).add_modifier(Modifier::UNDERLINED),
                    "outline" => style.fg(ansi.cursor).add_modifier(Modifier::BOLD),
                    _ => style.fg(ansi.cursor).add_modifier(Modifier::REVERSED),
                };
            }
            if text.starts_with('#') {
                style = style.fg(ansi.error);
            }
            if slot.is_some_and(|s| s.flags.stale()) {
                style = style.add_modifier(Modifier::DIM);
            }
            if let Some(region) = spill.region_at(sheet, row, col) {
                let on_edge = row == region.origin.row
                    || col == region.origin.col
                    || row
                        == region
                            .origin
                            .row
                            .saturating_add(region.rows.saturating_sub(1))
                    || col
                        == region
                            .origin
                            .col
                            .saturating_add((region.cols.saturating_sub(1)) as u16);
                if on_edge {
                    style = style.add_modifier(Modifier::UNDERLINED);
                }
            }
            let display = if slot.is_some_and(|s| s.flags.stale()) {
                hatch(&pad(&text, width, align), hbar)
            } else {
                pad(&text, width, align)
            };
            spans.push(Span::styled(display, style));
            col_i += 1 + overflow;
        }
        lines.push(Line::from(spans));
        if lines.len() as u16 >= area.height {
            break;
        }
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn overflow_cols(
    wb: &Workbook,
    sheet: SheetId,
    row: u32,
    rest: &[u16],
    text_len: usize,
    col_chars: usize,
    align: Align,
) -> usize {
    if !matches!(align, Align::Left) || text_len <= col_chars {
        return 0;
    }
    let mut extra = 0usize;
    let mut covered = col_chars;
    for next in rest {
        if covered >= text_len {
            break;
        }
        if !cell_empty(wb, sheet, row, *next) {
            break;
        }
        extra += 1;
        covered += col_chars + 1;
    }
    extra
}

fn cell_empty(wb: &Workbook, sheet: SheetId, row: u32, col: u16) -> bool {
    match wb.get(sheet, row, col) {
        Ok(Some(slot)) => matches!(slot.value, Value::Empty) && slot.formula.is_none(),
        _ => true,
    }
}

fn hatch(text: &str, mark: char) -> String {
    text.chars()
        .enumerate()
        .map(|(i, c)| if i % 2 == 1 { mark } else { c })
        .collect()
}

fn visible_cols(vp: &Viewport, first: u16, last: u16) -> Vec<u16> {
    let mut cols: Vec<u16> = (0..vp.freeze.cols).collect();
    for col in first..=last {
        if !cols.contains(&col) {
            cols.push(col);
        }
    }
    cols
}

#[derive(Clone, Copy)]
enum Align {
    Left,
    Right,
    Center,
}

fn pad(text: &str, width: usize, align: Align) -> String {
    let mut chars: Vec<char> = text.chars().take(width).collect();
    while chars.len() < width {
        match align {
            Align::Left => chars.push(' '),
            Align::Right => chars.insert(0, ' '),
            Align::Center => {
                if chars.len() % 2 == 0 {
                    chars.insert(0, ' ');
                } else {
                    chars.push(' ');
                }
            }
        }
    }
    chars.into_iter().collect()
}

fn cell_text(wb: &Workbook, sheet: SheetId, row: u32, col: u16, formulas: bool) -> (String, Align) {
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
            let formatted = format(FormatValue::Number(n), code.as_ref(), LocaleId::EN_US);
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

fn draw_status(
    frame: &mut Frame<'_>,
    area: Rect,
    ui: &UiSession,
    wb: &Workbook,
    theme_name: &str,
    ids: &[String],
    message: Option<&str>,
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
    let zoom = format!("{}%", (ui.viewport().zoom * 100.0).round());
    let dirty = if ui.undo_history().entries.is_empty() {
        ""
    } else {
        "*"
    };
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
        text.push_str(msg);
    }
    frame.render_widget(Paragraph::new(text), area);
}

fn draw_palette(frame: &mut Frame<'_>, area: Rect, palette: &Palette, unicode: bool) {
    let popup = centered(area, 60, 12);
    frame.render_widget(Clear, popup);
    let items: Vec<ListItem<'_>> = palette
        .hits
        .iter()
        .take(8)
        .map(|h| ListItem::new(format!("{}  {}", h.id, h.doc)))
        .collect();
    let title = if let Some(prompt) = &palette.prompt {
        prompt.clone()
    } else {
        format!("palette: {}", palette.query)
    };
    frame.render_widget(List::new(items).block(chrome_block(title, unicode)), popup);
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
    let body = match id {
        "find" => {
            let f = ui.find_replace();
            format!("find: {}\nreplace: {}", f.find, f.replace)
        }
        "goto" => format!("goto: {}", ui.goto().target),
        "keys" => "F1 keys overlay\nEsc closes panels\nCtrl+Q quits".into(),
        "changeset" => "changeset review (WP-07a store)".into(),
        "comments" => "comments (WP-19)".into(),
        "format" => "format panel (WP-18)".into(),
        "sort" | "filter" => format!("{id} panel (WP-17)"),
        other => format!("{other} panel"),
    };
    frame.render_widget(Paragraph::new(body).block(chrome_block(id, unicode)), popup);
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
        for x in area.x..area.x.saturating_add(area.width) {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}
