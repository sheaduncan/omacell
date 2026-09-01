//! Toolkit-neutral proposed-changeset review state.

use std::collections::BTreeMap;

use omacell_bus::{CellPreview, ChangePreview, ChangePreviewItem};
use omacell_core::changeset::{ChangeSummary, ChangesetId, CommandCall};
use omacell_core::command::Origin;

type CellIndex = BTreeMap<String, BTreeMap<(u32, u16), (usize, usize)>>;

/// Cell marker used by GUI/TUI in-place proposal overlays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewCellMark {
    /// Item is currently accepted.
    pub accepted: bool,
    /// Item owns the review cursor.
    pub selected: bool,
    /// Formula-bar input before application.
    pub before: Option<String>,
    /// Formula-bar input after application.
    pub after: Option<String>,
    /// Stored style changes even if formula-bar text is unchanged.
    pub style_changed: bool,
}

/// One independently accepted or rejected command.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewItem {
    /// Dry-run command and effects.
    pub preview: ChangePreviewItem,
    /// Included when the proposal is applied.
    pub accepted: bool,
}

/// Retained keyboard review state for one proposal.
#[derive(Clone, Debug, PartialEq)]
pub struct ChangesetReview {
    /// Proposal id.
    pub id: ChangesetId,
    /// Trusted model or agent origin.
    pub origin: Origin,
    /// Whole-proposal summary.
    pub summary: ChangeSummary,
    /// Reviewable commands in execution order.
    pub items: Vec<ReviewItem>,
    /// Active item.
    pub selected: usize,
    cell_index: CellIndex,
}

impl From<ChangePreview> for ChangesetReview {
    fn from(preview: ChangePreview) -> Self {
        let items = preview
            .items
            .into_iter()
            .map(|preview| ReviewItem {
                preview,
                accepted: true,
            })
            .collect::<Vec<_>>();
        let mut cell_index = BTreeMap::new();
        for (item_index, item) in items.iter().enumerate() {
            for (cell_index_in_item, cell) in item.preview.cells.iter().enumerate() {
                cell_index
                    .entry(cell.sheet.clone())
                    .or_insert_with(BTreeMap::new)
                    .entry((cell.row, cell.col))
                    .or_insert((item_index, cell_index_in_item));
            }
        }
        Self {
            id: preview.id,
            origin: preview.origin,
            summary: preview.summary,
            items,
            selected: 0,
            cell_index,
        }
    }
}

impl ChangesetReview {
    /// Move the review cursor, clamped to existing items.
    pub fn move_selection(&mut self, delta: i32) {
        if self.items.is_empty() {
            self.selected = 0;
            return;
        }
        self.selected = self
            .selected
            .saturating_add_signed(delta as isize)
            .min(self.items.len() - 1);
    }

    /// Toggle the active item.
    pub fn toggle_selected(&mut self) {
        if let Some(item) = self.items.get_mut(self.selected) {
            item.accepted = !item.accepted;
        }
    }

    /// Accept every command.
    pub fn accept_all(&mut self) {
        for item in &mut self.items {
            item.accepted = true;
        }
    }

    /// Reject every command.
    pub fn reject_all(&mut self) {
        for item in &mut self.items {
            item.accepted = false;
        }
    }

    /// Accepted commands, preserving model order.
    #[must_use]
    pub fn accepted_calls(&self) -> Vec<CommandCall> {
        self.items
            .iter()
            .filter(|item| item.accepted)
            .map(|item| item.preview.command.clone())
            .collect()
    }

    /// In-place overlay data for one grid cell.
    #[must_use]
    pub fn cell_mark(&self, sheet: &str, row: u32, col: u16) -> Option<ReviewCellMark> {
        let &(item_index, cell_index) = self.cell_index.get(sheet)?.get(&(row, col))?;
        let item = self.items.get(item_index)?;
        let cell = item.preview.cells.get(cell_index)?;
        Some(mark(cell, item.accepted, item_index == self.selected))
    }

    /// Human-readable panel body with keyboard affordances.
    #[must_use]
    pub fn body(&self) -> String {
        let accepted = self.items.iter().filter(|item| item.accepted).count();
        let mut lines = vec![
            format!(
                "{} · {accepted}/{} accepted",
                self.summary.text,
                self.items.len()
            ),
            String::new(),
        ];
        for (index, item) in self.items.iter().enumerate() {
            let cursor = if index == self.selected { '›' } else { ' ' };
            let checked = if item.accepted { 'x' } else { ' ' };
            lines.push(format!(
                "{cursor} [{checked}] {} — {}",
                item.preview.command.id, item.preview.summary.text
            ));
            for cell in item.preview.cells.iter().take(4) {
                lines.push(format!(
                    "      {}!{}  {} → {}{}",
                    cell.sheet,
                    cell_a1(cell.row, cell.col),
                    display_value(cell.before.as_deref()),
                    display_value(cell.after.as_deref()),
                    if cell.style_changed { "  [style]" } else { "" }
                ));
            }
            if item.preview.cells.len() > 4 {
                lines.push(format!(
                    "      … {} more changed cells",
                    item.preview.cells.len() - 4
                ));
            }
        }
        lines.push(String::new());
        lines.push(
            "↑/↓ select · Space toggle · A accept all · R reject all · Enter apply · Esc close"
                .into(),
        );
        lines.join("\n")
    }
}

fn mark(cell: &CellPreview, accepted: bool, selected: bool) -> ReviewCellMark {
    ReviewCellMark {
        accepted,
        selected,
        before: cell.before.clone(),
        after: cell.after.clone(),
        style_changed: cell.style_changed,
    }
}

fn display_value(value: Option<&str>) -> &str {
    value.filter(|value| !value.is_empty()).unwrap_or("∅")
}

fn cell_a1(row: u32, col: u16) -> String {
    let column = omacell_core::addr::col_to_letters(col).unwrap_or_else(|_| "?".into());
    format!("{column}{}", row + 1)
}
