//! Sheet tabs, formula bar, status line, palette, and docked panels.

use egui::{RichText, TextEdit, Ui};
use omacell_core::addr::{SheetId, col_to_letters};
use omacell_core::workbook::Workbook;
use omacell_ui::{EditSurface, Palette, PanelState, StatusLine, UiSession};

use crate::grid;
use crate::theme::GuiTheme;

/// Top sheet tabs.
pub fn tabs(ui: &mut Ui, wb: &Workbook, session: &UiSession, theme: &GuiTheme) -> Option<SheetId> {
    let mut selected = None;
    ui.horizontal(|ui| {
        for sheet in wb.sheets() {
            if !sheet.visibility.is_visible() {
                continue;
            }
            let active = sheet.id == session.selection().sheet;
            let text = if active {
                RichText::new(&sheet.name)
                    .color(theme.header_foreground)
                    .strong()
                    .underline()
            } else {
                RichText::new(&sheet.name).color(theme.muted)
            };
            if ui.add(egui::Button::new(text).frame(false)).clicked() {
                selected = Some(sheet.id);
            }
        }
    });
    selected
}

/// Formula bar. Returns new buffer text when the user edits it.
pub fn formula_bar(
    ui: &mut Ui,
    wb: &Workbook,
    session: &UiSession,
    theme: &GuiTheme,
) -> Option<String> {
    let sel = session.selection();
    let addr = format!(
        "{}{}",
        col_to_letters(sel.cursor.col).unwrap_or_else(|_| "A".into()),
        sel.cursor.row + 1
    );
    let mut text = grid::formula_text(wb, session);
    let mut changed = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new(addr).color(theme.header_foreground).strong());
        ui.label(RichText::new("fx").color(theme.muted));
        let editor = if session.formula_bar_expanded() {
            TextEdit::multiline(&mut text).desired_rows(3)
        } else {
            TextEdit::singleline(&mut text)
        };
        let response = ui.add(
            editor
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Monospace)
                .hint_text("Enter a value or formula"),
        );
        if response.gained_focus() && session.edit().is_idle() {
            session.begin_edit(EditSurface::FormulaBar, &text);
        }
        if response.changed() && !session.edit().is_idle() {
            changed = Some(text.clone());
        }
    });
    changed
}

/// Status line from `[layout] status_line`.
pub fn status(
    ui: &mut Ui,
    wb: &Workbook,
    session: &UiSession,
    theme: &GuiTheme,
    dirty: bool,
    message: Option<&str>,
    busy: bool,
) {
    let sel = session.selection();
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
    let ids = session.config().layout.status_line;
    line.refresh(
        &ids,
        session.mode().label(),
        &cell,
        &stats,
        calc,
        &theme.name,
    );
    let cfg = session.config();
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
        session.config().ai.agent.diagnose_offers,
        session.agent_visible(),
    ));
    let zoom = format!("{}%", (session.viewport().zoom * 100.0).round());
    for seg in &mut line.segments {
        match seg.id.as_str() {
            "zoom" => seg.text = zoom.clone(),
            "dirty" => seg.text = if dirty { "•" } else { "" }.into(),
            _ => {}
        }
    }
    ui.horizontal(|ui| {
        for seg in &line.segments {
            if seg.text.is_empty() {
                continue;
            }
            ui.label(RichText::new(&seg.text).color(theme.muted));
            ui.separator();
        }
        if busy {
            ui.label(RichText::new("working…").color(theme.warning));
        }
        if let Some(msg) = message {
            ui.label(RichText::new(msg).color(theme.foreground));
        }
    });
}

/// Command palette overlay.
pub fn palette(
    ctx: &egui::Context,
    palette: &Palette,
    selected: usize,
    theme: &GuiTheme,
) -> Option<String> {
    let mut chosen = None;
    egui::Window::new("palette")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_TOP, [0.0, 48.0])
        .frame(egui::Frame::popup(&ctx.style()).fill(theme.popup_background))
        .show(ctx, |ui| {
            ui.label(palette.prompt.as_deref().unwrap_or("palette"));
            ui.label(&palette.query);
            for (i, hit) in palette.hits.iter().take(12).enumerate() {
                let text = format!("{}  {}", hit.id, hit.doc);
                let rich = if i == selected {
                    RichText::new(text).background_color(theme.selection)
                } else {
                    RichText::new(text)
                };
                if ui.selectable_label(i == selected, rich).clicked() {
                    chosen = Some(hit.id.clone());
                }
            }
        });
    chosen
}

/// Docked panel.
pub fn panel(ui: &mut Ui, panel: &PanelState, session: &UiSession, theme: &GuiTheme) {
    let Some(id) = &panel.visible else {
        return;
    };
    let body = match id.as_str() {
        "find" => {
            let f = session.find_replace();
            format!("find: {}\nreplace: {}", f.find, f.replace)
        }
        "goto" => format!("goto: {}", session.goto().target),
        "keys" => "F1 keys overlay\nEsc closes panels\nCtrl+Q quits".into(),
        "changeset" => "changeset review (WP-07a store)".into(),
        "format" => "format panel (WP-18)".into(),
        other => format!("{other} panel"),
    };
    let width = (panel.width as f32 / 8.0).clamp(180.0, 360.0);
    match panel.side.as_str() {
        "left" => {
            egui::SidePanel::left("omacell-panel")
                .resizable(true)
                .min_width(width)
                .show_inside(ui, |ui| {
                    ui.heading(id);
                    ui.label(RichText::new(body).color(theme.foreground));
                });
        }
        "bottom" => {
            egui::TopBottomPanel::bottom("omacell-panel")
                .resizable(true)
                .show_inside(ui, |ui| {
                    ui.heading(id);
                    ui.label(body);
                });
        }
        _ => {
            egui::SidePanel::right("omacell-panel")
                .resizable(true)
                .min_width(width)
                .show_inside(ui, |ui| {
                    ui.heading(id);
                    ui.label(body);
                });
        }
    }
}

/// Optional classic menu bar.
pub fn menu_bar(ui: &mut Ui) -> Option<&'static str> {
    let mut cmd = None;
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            if ui.button("Save").clicked() {
                cmd = Some("file.save");
            }
        });
        ui.menu_button("Edit", |ui| {
            if ui.button("Undo").clicked() {
                cmd = Some("edit.undo");
            }
            if ui.button("Copy").clicked() {
                cmd = Some("edit.copy");
            }
        });
        ui.menu_button("Help", |ui| {
            if ui.button("Keys").clicked() {
                cmd = Some("help.keys");
            }
        });
    });
    cmd
}
