//! Sheet tabs, formula bar, status line, palette, and docked panels.

use egui::{RichText, TextEdit, Ui};
use omacell_core::addr::{SheetId, col_to_letters};
use omacell_core::workbook::Workbook;
use omacell_ui::{EditSurface, Palette, PanelState, StatusLine, UiSession};

use crate::grid;
use crate::i18n::tr;
use crate::theme::GuiTheme;

/// Command produced by an actionable status-line segment.
pub struct StatusAction {
    /// Registered command id.
    pub command: &'static str,
    /// Command arguments.
    pub args: serde_json::Value,
}

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
        ui.label(RichText::new(tr("formula-symbol")).color(theme.muted));
        let editor = if session.formula_bar_expanded() {
            TextEdit::multiline(&mut text).desired_rows(3)
        } else {
            TextEdit::singleline(&mut text)
        };
        let response = ui.add(
            editor
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Monospace)
                .hint_text(tr("formula-hint")),
        );
        if response.gained_focus() && session.edit().is_idle() {
            session.begin_edit(EditSurface::FormulaBar, &text);
        }
        if response.changed() && !session.edit().is_idle() {
            changed = Some(text.clone());
        }
        if let Some(ghost) = session.edit().ghost {
            ui.label(RichText::new(ghost).color(theme.muted).monospace());
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
) -> Option<StatusAction> {
    let sel = session.selection();
    let cell = format!(
        "{}{}",
        col_to_letters(sel.cursor.col).unwrap_or_else(|_| "A".into()),
        sel.cursor.row + 1
    );
    let stats = format!("{} {}", tr("status-count"), sel.cell_count());
    let calc = match wb.settings().calc_mode {
        omacell_core::workbook::CalcMode::Manual => tr("status-manual"),
        omacell_core::workbook::CalcMode::AutomaticExceptTables => tr("status-auto-except-tables"),
        omacell_core::workbook::CalcMode::Automatic => tr("status-auto"),
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
    let mut picked = None;
    ui.horizontal(|ui| {
        for seg in &line.segments {
            if seg.text.is_empty() {
                continue;
            }
            if let Some(action) = status_action(&seg.id) {
                let label = status_accessible_label(&seg.id, &seg.text);
                let response =
                    ui.add(egui::Button::new(RichText::new(label).color(theme.muted)).frame(false));
                if response.clicked() {
                    picked = Some(action);
                }
            } else {
                ui.label(RichText::new(&seg.text).color(theme.muted));
            }
            ui.separator();
        }
        if busy {
            ui.label(RichText::new(tr("status-working")).color(theme.warning));
        }
        if let Some(msg) = message {
            ui.label(RichText::new(msg).color(theme.foreground));
        }
    });
    picked
}

fn status_accessible_label(id: &str, text: &str) -> String {
    let prefix = match id {
        "mode" => tr("status-mode"),
        "cell" => tr("status-cell"),
        "calc" => tr("status-calculation"),
        "theme" => tr("status-theme"),
        "zoom" => tr("status-zoom"),
        "dirty" => tr("status-save"),
        _ => return text.to_string(),
    };
    format!("{prefix} {text}")
}

/// Map a status segment onto an existing registered command.
#[must_use]
pub fn status_action(id: &str) -> Option<StatusAction> {
    let (command, args) = match id {
        "mode" => ("mode.normal", serde_json::json!({})),
        "cell" => ("nav.goto", serde_json::json!({})),
        "calc" => ("calc.recalc", serde_json::json!({})),
        "theme" => ("theme.reload", serde_json::json!({})),
        "zoom" => ("view.zoom", serde_json::json!({"factor": 1.0})),
        "dirty" => ("file.save", serde_json::json!({})),
        "ai" => ("ai.agent", serde_json::json!({})),
        "diagnose" => ("ai.agent", serde_json::json!({"diagnose": true})),
        _ => return None,
    };
    Some(StatusAction { command, args })
}

/// Command palette overlay.
pub fn palette(
    ctx: &egui::Context,
    palette: &Palette,
    selected: usize,
    theme: &GuiTheme,
) -> Option<String> {
    let mut chosen = None;
    egui::Window::new(tr("palette-title"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_TOP, [0.0, 48.0])
        .frame(egui::Frame::popup(&ctx.style()).fill(theme.popup_background))
        .show(ctx, |ui| {
            ui.label(palette.prompt.as_deref().unwrap_or(tr("palette-prompt")));
            ui.label(&palette.query);
            if let Some(preview) = &palette.preview {
                ui.monospace(preview);
            }
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
    let body = panel.body.clone().unwrap_or_else(|| match id.as_str() {
        "find" => {
            let f = session.find_replace();
            format!(
                "{}: {}\n{}: {}",
                tr("panel-find"),
                f.find,
                tr("panel-replace"),
                f.replace
            )
        }
        "goto" => format!("{}: {}", tr("panel-goto"), session.goto().target),
        "keys" => format!(
            "{}\n{}\n{}",
            tr("panel-keys-help"),
            tr("panel-escape-help"),
            tr("panel-quit-help")
        ),
        "changeset" => session
            .changeset_review()
            .map_or_else(|| tr("panel-no-changesets").into(), |review| review.body()),
        "agent" => session.agent_panel().body(),
        "formula" => {
            let mut body = session.formula_assist().map_or_else(
                || tr("panel-no-formula-assist").into(),
                |assist| assist.body(),
            );
            if let Some(review) = session.changeset_review() {
                body.push_str("\n\n");
                body.push_str(&review.body());
            }
            body
        }
        "import" => session
            .import_review()
            .map_or_else(|| tr("panel-no-import").into(), |review| review.body()),
        "format" => tr("panel-format-help").into(),
        "comments" => tr("panel-comments-help").into(),
        "sort" => tr("panel-sort-help").into(),
        "filter" => tr("panel-filter-help").into(),
        other => format!("{other} {}", tr("panel-suffix")),
    });
    let title = match id.as_str() {
        "goto" => tr("panel-goto"),
        "keys" => tr("panel-keys-title"),
        "print" => tr("panel-print-title"),
        other => other,
    };
    let width = (panel.width as f32 / 8.0).clamp(180.0, 360.0);
    match panel.side.as_str() {
        "left" => {
            egui::SidePanel::left("omacell-panel")
                .resizable(true)
                .min_width(width)
                .show_inside(ui, |ui| {
                    ui.heading(title);
                    ui.label(RichText::new(body).color(theme.foreground));
                });
        }
        "bottom" => {
            egui::TopBottomPanel::bottom("omacell-panel")
                .resizable(true)
                .show_inside(ui, |ui| {
                    ui.heading(title);
                    ui.label(body);
                });
        }
        _ => {
            egui::SidePanel::right("omacell-panel")
                .resizable(true)
                .min_width(width)
                .show_inside(ui, |ui| {
                    ui.heading(title);
                    ui.label(body);
                });
        }
    }
}

#[cfg(test)]
mod panel_tests {
    use super::panel_width_points;

    #[test]
    fn panel_width_is_already_expressed_in_css_pixels() {
        assert_eq!(panel_width_points(360), 360.0);
        assert_eq!(panel_width_points(1), 180.0);
        assert_eq!(panel_width_points(u32::MAX), 360.0);
    }
}

/// Optional classic menu bar.
pub fn menu_bar(ui: &mut Ui) -> Option<&'static str> {
    let mut cmd = None;
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button(tr("menu-file"), |ui| {
            if ui.button(tr("menu-save")).clicked() {
                cmd = Some("file.save");
            }
        });
        ui.menu_button(tr("menu-edit"), |ui| {
            if ui.button(tr("menu-undo")).clicked() {
                cmd = Some("edit.undo");
            }
            if ui.button(tr("menu-copy")).clicked() {
                cmd = Some("edit.copy");
            }
        });
        ui.menu_button(tr("menu-help"), |ui| {
            if ui.button(tr("menu-keys")).clicked() {
                cmd = Some("help.keys");
            }
        });
    });
    cmd
}

#[cfg(test)]
mod tests {
    use super::status_action;

    #[test]
    fn clickable_status_segments_use_registered_commands() {
        let zoom = status_action("zoom").expect("zoom action");
        assert_eq!(zoom.command, "view.zoom");
        assert_eq!(zoom.args, serde_json::json!({"factor": 1.0}));
        let diagnose = status_action("diagnose").expect("diagnose action");
        assert_eq!(diagnose.command, "ai.agent");
        assert_eq!(diagnose.args, serde_json::json!({"diagnose": true}));
        assert!(status_action("stats").is_none());
    }
}
