//! In-cell and formula-bar editing (F-5.2, F-5.3).

use omacell_core::addr::{CellRef, col_to_letters};
use omacell_core::formula::{Expr, ExprKind, parse_editor, print};

use crate::error;
use omacell_core::error::CoreError;

/// Where editing is happening. Both share this state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EditSurface {
    /// Not editing.
    #[default]
    Idle,
    /// In-cell (`F2` / `i`).
    InCell,
    /// Formula bar.
    FormulaBar,
}

/// Live edit buffer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditState {
    /// Surface.
    pub surface: EditSurface,
    /// Formula-bar text (canonical, not localized).
    pub buffer: String,
    /// UTF-8 cursor byte offset.
    pub cursor: usize,
    /// Cell being edited.
    pub origin: Option<CellRef>,
    /// True when the next navigation inserts a reference.
    pub point: bool,
}

impl EditState {
    /// Start editing `origin` with `initial` text.
    pub fn begin(&mut self, surface: EditSurface, origin: CellRef, initial: &str) {
        self.surface = surface;
        self.buffer = initial.to_string();
        self.cursor = self.buffer.len();
        self.origin = Some(origin);
        self.point = looks_like_formula(&self.buffer) && point_ready(&self.buffer);
    }

    /// Idle?
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.surface == EditSurface::Idle
    }

    /// Insert a character at the cursor.
    pub fn insert_char(&mut self, c: char) {
        self.buffer.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.point = looks_like_formula(&self.buffer) && point_ready(&self.buffer);
    }

    /// Cancel and return to idle, restoring nothing (caller still holds the cell).
    pub fn cancel(&mut self) {
        *self = Self::default();
    }

    /// Commit: return the buffer and go idle.
    pub fn commit(&mut self) -> String {
        let text = std::mem::take(&mut self.buffer);
        *self = Self::default();
        text
    }

    /// Insert an A1 reference at the cursor (point mode).
    pub fn insert_ref(&mut self, cell: CellRef) -> Result<(), CoreError> {
        if !self.point {
            return Err(error::edit("not in point mode"));
        }
        let text = a1(&cell);
        self.buffer.insert_str(self.cursor, &text);
        self.cursor += text.len();
        self.point = point_ready(&self.buffer);
        Ok(())
    }

    /// Excel `F4` cycle on the reference containing the cursor.
    pub fn cycle_anchor(&mut self) -> Result<(), CoreError> {
        if self.is_idle() {
            return Err(error::edit("F4 requires an active edit"));
        }
        let parsed = parse_editor(&self.buffer);
        let Some(expr) = parsed.expr else {
            return Ok(());
        };
        let cursor = self.cursor as u32;
        let mut target: Option<(u32, u32, CellRef)> = None;
        collect_ref(&expr, cursor, &mut target);
        let Some((start, end, mut cell)) = target else {
            return Ok(());
        };
        cycle_abs(&mut cell);
        let replacement = a1(&cell);
        let start = start as usize;
        let end = (end as usize).min(self.buffer.len());
        self.buffer.replace_range(start..end, &replacement);
        self.cursor = start + replacement.len();
        let _ = print;
        Ok(())
    }

    /// Colorization spans for formula references (`references.0`..`7`).
    #[must_use]
    pub fn reference_spans(&self) -> Vec<(usize, usize, usize)> {
        if !looks_like_formula(&self.buffer) {
            return Vec::new();
        }
        let parsed = parse_editor(&self.buffer);
        let Some(expr) = parsed.expr else {
            return Vec::new();
        };
        let mut spans = Vec::new();
        gather_spans(&expr, &mut spans);
        spans
            .into_iter()
            .enumerate()
            .map(|(i, (a, b))| (a, b, i % 8))
            .collect()
    }
}

fn looks_like_formula(s: &str) -> bool {
    s.starts_with('=')
}

fn point_ready(s: &str) -> bool {
    let t = s.trim_end();
    t.ends_with('=')
        || t.ends_with('(')
        || t.ends_with(',')
        || t.ends_with('+')
        || t.ends_with('-')
        || t.ends_with('*')
        || t.ends_with('/')
        || t.ends_with('^')
        || t.ends_with('&')
}

fn a1(cell: &CellRef) -> String {
    let col = col_to_letters(cell.col).unwrap_or_else(|_| "A".into());
    let mut s = String::new();
    if cell.col_abs {
        s.push('$');
    }
    s.push_str(&col);
    if cell.row_abs {
        s.push('$');
    }
    s.push_str(&(cell.row + 1).to_string());
    s
}

/// Excel cycle: A1 → $A$1 → A$1 → $A1 → A1.
fn cycle_abs(cell: &mut CellRef) {
    match (cell.col_abs, cell.row_abs) {
        (false, false) => {
            cell.col_abs = true;
            cell.row_abs = true;
        }
        (true, true) => {
            cell.col_abs = false;
            cell.row_abs = true;
        }
        (false, true) => {
            cell.col_abs = true;
            cell.row_abs = false;
        }
        (true, false) => {
            cell.col_abs = false;
            cell.row_abs = false;
        }
    }
}

fn collect_ref(expr: &Expr, cursor: u32, out: &mut Option<(u32, u32, CellRef)>) {
    expr.walk(&mut |node| {
        if node.span.start > cursor || cursor > node.span.end {
            return;
        }
        match &node.kind {
            ExprKind::Cell { cell, .. } => {
                *out = Some((node.span.start, node.span.end, *cell));
            }
            ExprKind::Range { range, .. } => {
                *out = Some((node.span.start, node.span.end, range.start));
            }
            _ => {}
        }
    });
}

fn gather_spans(expr: &Expr, out: &mut Vec<(usize, usize)>) {
    expr.walk(&mut |node| {
        if matches!(node.kind, ExprKind::Cell { .. } | ExprKind::Range { .. }) {
            out.push((node.span.start as usize, node.span.end as usize));
        }
    });
}
