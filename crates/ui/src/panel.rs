//! Docked panel model (one visible; Esc returns focus).

use omacell_core::error::CoreError;
use omacell_core::style::{Style, Underline};
use omacell_core::workbook::Workbook;
use serde::Deserialize;

use crate::selection::Selection;

/// Known panel ids.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PanelState {
    /// Currently visible panel (`format`, `find`, `changeset`, …).
    pub visible: Option<String>,
    /// Optional dynamic panel body supplied by a command.
    pub body: Option<String>,
    /// Dock side from config (`right` / `left` / `bottom`).
    pub side: String,
    /// Width in px.
    pub width: u32,
    /// Grid has focus when no panel (or after Esc).
    pub grid_focused: bool,
}

impl PanelState {
    /// Open `id`, taking focus.
    pub fn open(&mut self, id: &str) {
        self.visible = Some(id.to_string());
        self.body = None;
        self.grid_focused = false;
    }

    /// Open `id` with command-generated content, taking focus.
    pub fn open_with_body(&mut self, id: &str, body: impl Into<String>) {
        self.open(id);
        self.body = Some(body.into());
    }

    /// Esc: close and return focus to the grid.
    pub fn dismiss(&mut self) {
        self.visible = None;
        self.body = None;
        self.grid_focused = true;
    }
}

/// Workbook-backed panel whose content can be built from a reader snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkbookPanel {
    /// Notes and threaded comments on the active sheet.
    Comments,
    /// Sort command guidance for the active selection.
    Sort,
    /// Current AutoFilter state and filter command guidance.
    Filter,
}

impl WorkbookPanel {
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Comments => "comments",
            Self::Sort => "sort",
            Self::Filter => "filter",
        }
    }
}

/// Open a workbook-backed panel from an immutable reader snapshot.
pub(crate) fn open_workbook_panel(
    panel: &mut PanelState,
    selection: &Selection,
    workbook: &Workbook,
    kind: WorkbookPanel,
) {
    let body = match kind {
        WorkbookPanel::Comments => comments_body(workbook, selection),
        WorkbookPanel::Sort => sort_body(selection),
        WorkbookPanel::Filter => filter_body(workbook, selection),
    };
    panel.open_with_body(kind.id(), body);
}

/// Apply command output that belongs in a docked panel.
///
/// Returns `true` when `command` was recognized as a panel-producing command.
pub fn apply_command_panel(
    session: &crate::session::UiSession,
    command: &str,
    result: &serde_json::Value,
) -> Result<bool, CoreError> {
    if command != "format.panel" {
        return Ok(false);
    }
    let result: FormatPanelResult = serde_json::from_value(result.clone()).map_err(|error| {
        CoreError::new(
            "ui.panel",
            format!("format.panel returned an invalid result: {error}"),
        )
    })?;
    let mut panel = session.panel();
    panel.open_with_body("format", format_body(&result));
    session.set_panel(panel);
    Ok(true)
}

#[derive(Debug, Deserialize)]
struct FormatPanelResult {
    #[serde(default)]
    range: Option<String>,
    style: Style,
    #[serde(default)]
    number_format: Option<String>,
}

fn format_body(result: &FormatPanelResult) -> String {
    let style = &result.style;
    let mut traits = Vec::new();
    if style.font.bold {
        traits.push("bold");
    }
    if style.font.italic {
        traits.push("italic");
    }
    if style.font.underline != Underline::None {
        traits.push("underlined");
    }
    if style.font.strike {
        traits.push("struck");
    }
    let traits = if traits.is_empty() {
        "regular".to_string()
    } else {
        traits.join(", ")
    };
    let font = if style.font.name.is_empty() {
        "theme"
    } else {
        &style.font.name
    };
    let range = result.range.as_deref().unwrap_or("current selection");
    let number_format = result.number_format.as_deref().unwrap_or("General");
    format!(
        "Selection: {range}\nNumber format: {number_format}\nFont: {font} {:.1} pt ({traits}), {:?}\nFill: {:?}\nBorder: {:?}\nAlignment: {:?}\nProtection: {}{}",
        style.font.size_pt,
        style.font.color,
        style.fill,
        style.border,
        style.alignment,
        if style.protection.locked {
            "locked"
        } else {
            "unlocked"
        },
        if style.protection.hidden {
            ", formula hidden"
        } else {
            ""
        },
    )
}

fn comments_body(workbook: &Workbook, selection: &Selection) -> String {
    let Some(sheet) = workbook.sheet(selection.sheet) else {
        return "The selected sheet is no longer available.".into();
    };
    let mut entries = Vec::with_capacity(sheet.notes.len().saturating_add(sheet.comments.len()));
    for (&(row, col), note) in &sheet.notes {
        let author = note.author.as_deref().unwrap_or("unknown author");
        entries.push((
            row,
            col,
            0_u8,
            format!(
                "{}  note by {author}: {}",
                cell_a1(row, col),
                one_line(&note.text)
            ),
        ));
    }
    for (&(row, col), comment) in &sheet.comments {
        let state = if comment.resolved { "resolved" } else { "open" };
        let replies = match comment.replies.len() {
            0 => String::new(),
            1 => ", 1 reply".into(),
            count => format!(", {count} replies"),
        };
        entries.push((
            row,
            col,
            1_u8,
            format!(
                "{}  comment by {} ({state}{replies}): {}",
                cell_a1(row, col),
                comment.author,
                one_line(&comment.text)
            ),
        ));
    }
    entries.sort_by_key(|entry| (entry.0, entry.1, entry.2));
    if entries.is_empty() {
        return format!("{} has no notes or threaded comments.", sheet.name);
    }
    let total = entries.len();
    let mut body = format!("{} — {total} note/comment entr", sheet.name);
    body.push_str(if total == 1 { "y" } else { "ies" });
    for (_, _, _, entry) in entries.into_iter().take(100) {
        body.push('\n');
        body.push_str(&entry);
    }
    if total > 100 {
        body.push_str(&format!("\n… {} more", total - 100));
    }
    body.push_str("\n\nUse nav.goto to select an entry; edit.commentresolve resolves or reopens the selected thread.");
    body
}

fn sort_body(selection: &Selection) -> String {
    let range = selection.active().to_range().to_a1();
    format!(
        "Selection: {range}\n\nRun range.sort from the command palette. Keys use zero-based offsets inside the selection.\n\nExample:\n{{\"range\":\"{range}\",\"keys\":[{{\"offset\":0,\"descending\":false}}],\"header\":true}}"
    )
}

fn filter_body(workbook: &Workbook, selection: &Selection) -> String {
    let Some(sheet) = workbook.sheet(selection.sheet) else {
        return "The selected sheet is no longer available.".into();
    };
    let selected = selection.active().to_range().to_a1();
    let mut body = format!("Selection: {selected}\n");
    if let Some(filter) = &sheet.autofilter {
        body.push_str(&format!("Active filter: {}\n", filter.range.to_a1()));
        if filter.columns.is_empty() {
            body.push_str("Criteria: none\n");
        } else {
            body.push_str("Criteria:\n");
            for column in &filter.columns {
                body.push_str(&format!(
                    "- column {}: {:?}\n",
                    column.col_id, column.criteria
                ));
            }
        }
    } else {
        body.push_str("Active filter: none\n");
    }
    body.push_str(
        "\nfilter.toggle creates/removes a filter on the selection; filter.set applies criteria; filter.clear removes the active filter.",
    );
    body
}

fn cell_a1(row: u32, col: u16) -> String {
    match omacell_core::addr::col_to_letters(col) {
        Ok(letters) => format!("{letters}{}", row.saturating_add(1)),
        Err(_) => format!(
            "R{}C{}",
            row.saturating_add(1),
            u32::from(col).saturating_add(1)
        ),
    }
}

fn one_line(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut short = compact.chars().take(120).collect::<String>();
    if compact.chars().count() > 120 {
        short.push('…');
    }
    short
}

#[cfg(test)]
mod tests {
    use super::*;
    use omacell_core::sheet::{Comment, Note};

    #[test]
    fn workbook_panels_contain_live_snapshot_data() {
        let mut workbook = Workbook::new();
        let sheet = workbook.active_sheet();
        let selected = Selection::a1(sheet);
        workbook
            .set_note(
                sheet,
                1,
                2,
                Some(Note {
                    author: Some("Ada".into()),
                    text: "check this".into(),
                }),
            )
            .unwrap();
        workbook
            .set_comment(
                sheet,
                3,
                0,
                Some(Comment {
                    author: "Grace".into(),
                    text: "needs review".into(),
                    replies: Vec::new(),
                    resolved: false,
                }),
            )
            .unwrap();
        let mut panel = PanelState::default();

        open_workbook_panel(&mut panel, &selected, &workbook, WorkbookPanel::Comments);
        let body = panel.body.as_deref().unwrap();
        assert!(body.contains("C2  note by Ada"));
        assert!(body.contains("A4  comment by Grace (open)"));

        open_workbook_panel(&mut panel, &selected, &workbook, WorkbookPanel::Filter);
        assert!(
            panel
                .body
                .as_deref()
                .unwrap()
                .contains("Active filter: none")
        );
    }
}
