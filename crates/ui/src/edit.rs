//! In-cell and formula-bar editing (F-5.2, F-5.3).

use omacell_core::addr::{CellRef, col_to_letters};
use omacell_core::formula::{Expr, ExprKind, parse_editor};
use omacell_core::locale::LocaleSeparators;

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
    /// Model-proposed suffix rendered as ghost text and accepted with `Tab`.
    pub ghost: Option<String>,
}

/// Convert localized numeric and formula separators to canonical entry text.
#[must_use]
pub fn canonicalize_entry(input: &str, separators: LocaleSeparators) -> String {
    if !input.trim_start().starts_with('=') {
        let trimmed = input.trim();
        if let Some(normalized) = normalize_localized_number(trimmed, separators) {
            return normalized;
        }
        return input.to_string();
    }

    let mut canonical = String::with_capacity(input.len());
    let mut characters = input.chars().peekable();
    let mut quoted = false;
    while let Some(character) = characters.next() {
        if character == '"' {
            canonical.push(character);
            if quoted && characters.peek() == Some(&'"') {
                canonical.push(characters.next().unwrap_or('"'));
            } else {
                quoted = !quoted;
            }
            continue;
        }
        if quoted {
            canonical.push(character);
        } else if character == separators.list && separators.list != ',' {
            canonical.push(',');
        } else if character == separators.decimal && separators.decimal != '.' {
            canonical.push('.');
        } else {
            canonical.push(character);
        }
    }
    canonical
}

fn normalize_localized_number(text: &str, separators: LocaleSeparators) -> Option<String> {
    if text.is_empty() || separators.decimal == separators.thousands {
        return None;
    }
    let (mantissa, exponent) = match text.find(['e', 'E']) {
        Some(at) => {
            let exponent = &text[at + 1..];
            if exponent.is_empty()
                || text[at + 1..].contains(['e', 'E'])
                || exponent.parse::<i32>().is_err()
            {
                return None;
            }
            (&text[..at], Some(exponent))
        }
        None => (text, None),
    };
    let (sign, unsigned) = if let Some(rest) = mantissa.strip_prefix('+') {
        ("+", rest)
    } else if let Some(rest) = mantissa.strip_prefix('-') {
        ("-", rest)
    } else {
        ("", mantissa)
    };
    let mut decimal_parts = unsigned.split(separators.decimal);
    let integer = decimal_parts.next().unwrap_or("");
    let fraction = decimal_parts.next();
    if decimal_parts.next().is_some()
        || fraction.is_some_and(|digits| !digits.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    let groups = integer.split(separators.thousands).collect::<Vec<_>>();
    let grouped = groups.len() > 1;
    let leading_fraction =
        !grouped && integer.is_empty() && fraction.is_some_and(|digits| !digits.is_empty());
    if (!leading_fraction
        && groups.first().is_none_or(|group| {
            group.is_empty()
                || !group.bytes().all(|byte| byte.is_ascii_digit())
                || (grouped && group.len() > 3)
        }))
        || groups
            .iter()
            .skip(1)
            .any(|group| group.len() != 3 || !group.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    let mut normalized = format!("{sign}{}", groups.concat());
    if let Some(fraction) = fraction {
        normalized.push('.');
        normalized.push_str(fraction);
    }
    if let Some(exponent) = exponent {
        normalized.push('e');
        normalized.push_str(exponent);
    }
    normalized
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
        .map(|_| normalized)
}

impl EditState {
    /// Start editing `origin` with `initial` text.
    pub fn begin(&mut self, surface: EditSurface, origin: CellRef, initial: &str) {
        self.surface = surface;
        self.buffer = initial.to_string();
        self.cursor = self.buffer.len();
        self.origin = Some(origin);
        self.point = looks_like_formula(&self.buffer) && point_ready(&self.buffer);
        self.ghost = None;
    }

    /// Idle?
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.surface == EditSurface::Idle
    }

    /// Insert a character at the cursor.
    pub fn insert_char(&mut self, c: char) {
        self.ghost = None;
        self.clamp_cursor();
        self.buffer.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.point = looks_like_formula(&self.buffer) && point_ready(&self.buffer);
    }

    /// Insert text at the cursor.
    pub fn insert_text(&mut self, text: &str) {
        self.ghost = None;
        self.clamp_cursor();
        self.buffer.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.point = looks_like_formula(&self.buffer) && point_ready(&self.buffer);
    }

    /// Remove the character immediately before the cursor.
    pub fn backspace(&mut self) {
        self.ghost = None;
        self.clamp_cursor();
        let Some((start, _)) = self.buffer[..self.cursor].char_indices().next_back() else {
            return;
        };
        self.buffer.replace_range(start..self.cursor, "");
        self.cursor = start;
        self.update_point();
    }

    /// Remove the character at the cursor.
    pub fn delete_forward(&mut self) {
        self.ghost = None;
        self.clamp_cursor();
        let Some(ch) = self.buffer[self.cursor..].chars().next() else {
            return;
        };
        self.buffer
            .replace_range(self.cursor..self.cursor + ch.len_utf8(), "");
        self.update_point();
    }

    /// Move the caret one Unicode scalar to the left.
    pub fn move_left(&mut self) {
        self.ghost = None;
        self.clamp_cursor();
        if let Some((start, _)) = self.buffer[..self.cursor].char_indices().next_back() {
            self.cursor = start;
        }
    }

    /// Move the caret one Unicode scalar to the right.
    pub fn move_right(&mut self) {
        self.ghost = None;
        self.clamp_cursor();
        if let Some(ch) = self.buffer[self.cursor..].chars().next() {
            self.cursor += ch.len_utf8();
        }
    }

    /// Move the caret to the same character column on the previous line.
    pub fn move_up(&mut self) {
        self.ghost = None;
        self.clamp_cursor();
        let line_start = self.buffer[..self.cursor]
            .rfind('\n')
            .map_or(0, |newline| newline + 1);
        if line_start == 0 {
            return;
        }
        let column = self.buffer[line_start..self.cursor].chars().count();
        let previous_end = line_start - 1;
        let previous_start = self.buffer[..previous_end]
            .rfind('\n')
            .map_or(0, |newline| newline + 1);
        self.cursor = byte_at_char_column(&self.buffer, previous_start, previous_end, column);
    }

    /// Move the caret to the same character column on the next line.
    pub fn move_down(&mut self) {
        self.ghost = None;
        self.clamp_cursor();
        let line_start = self.buffer[..self.cursor]
            .rfind('\n')
            .map_or(0, |newline| newline + 1);
        let column = self.buffer[line_start..self.cursor].chars().count();
        let Some(next_start) = self.buffer[self.cursor..]
            .find('\n')
            .map(|newline| self.cursor + newline + 1)
        else {
            return;
        };
        let next_end = self.buffer[next_start..]
            .find('\n')
            .map_or(self.buffer.len(), |newline| next_start + newline);
        self.cursor = byte_at_char_column(&self.buffer, next_start, next_end, column);
    }

    /// Move the caret to the start of the edit buffer.
    pub fn move_home(&mut self) {
        self.ghost = None;
        self.cursor = 0;
    }

    /// Move the caret to the end of the edit buffer.
    pub fn move_end(&mut self) {
        self.ghost = None;
        self.cursor = self.buffer.len();
    }

    /// Replace text supplied by a toolkit editor and reset completion state.
    pub fn replace_from_toolkit(&mut self, text: String) {
        self.buffer = text;
        self.cursor = self.buffer.len();
        self.ghost = None;
        self.update_point();
    }

    /// Retain a completion only when it still matches the current edit prefix.
    pub fn set_ghost(&mut self, prefix: &str, completion: &str) -> bool {
        self.ghost = None;
        if self.is_idle()
            || self.buffer != prefix
            || self.cursor != self.buffer.len()
            || completion.len() > 4_096
            || completion.chars().any(char::is_control)
        {
            return false;
        }
        let suffix = completion.strip_prefix(prefix).unwrap_or(completion);
        if suffix.is_empty() {
            return false;
        }
        self.ghost = Some(suffix.to_string());
        true
    }

    /// Insert the retained ghost suffix without committing the cell.
    pub fn accept_ghost(&mut self) -> bool {
        let Some(ghost) = self.ghost.take() else {
            return false;
        };
        self.insert_text(&ghost);
        true
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
        self.clamp_cursor();
        self.ghost = None;
        let text = a1(&cell)?;
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
        self.ghost = None;
        let parsed = parse_editor(&self.buffer);
        let Some(expr) = parsed.expr else {
            return Ok(());
        };
        let cursor = self.cursor as u32;
        let mut target = None;
        collect_ref(&expr, cursor, &mut target);
        let Some(target) = target else {
            return Ok(());
        };
        let (start, end, replacement) = match target {
            AnchorTarget::Cell {
                start,
                end,
                mut cell,
            } => {
                cycle_abs(&mut cell);
                let prefix = reference_prefix(&self.buffer, start, end);
                (start, end, format!("{prefix}{}", a1(&cell)?))
            }
            AnchorTarget::Range {
                start,
                end,
                mut range,
            } => {
                let prefix = reference_prefix(&self.buffer, start, end);
                let body = cycle_range(&mut range)?;
                (start, end, format!("{prefix}{body}"))
            }
        };
        self.buffer.replace_range(start..end, &replacement);
        self.cursor = start + replacement.len();
        Ok(())
    }

    fn clamp_cursor(&mut self) {
        self.cursor = self.cursor.min(self.buffer.len());
        while !self.buffer.is_char_boundary(self.cursor) {
            self.cursor = self.cursor.saturating_sub(1);
        }
    }

    fn update_point(&mut self) {
        self.point = looks_like_formula(&self.buffer) && point_ready(&self.buffer);
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

fn reference_prefix(buffer: &str, start: usize, end: usize) -> &str {
    buffer
        .get(start..end)
        .and_then(|source| source.rfind('!').map(|at| &source[..=at]))
        .unwrap_or("")
}

fn byte_at_char_column(buffer: &str, start: usize, end: usize, column: usize) -> usize {
    buffer[start..end]
        .char_indices()
        .nth(column)
        .map_or(end, |(offset, _)| start + offset)
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

fn a1(cell: &CellRef) -> Result<String, CoreError> {
    let col = col_to_letters(cell.col)?;
    let mut s = String::new();
    if cell.col_abs {
        s.push('$');
    }
    s.push_str(&col);
    if cell.row_abs {
        s.push('$');
    }
    s.push_str(&(cell.row + 1).to_string());
    Ok(s)
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

#[derive(Clone, Copy)]
enum AnchorTarget {
    Cell {
        start: usize,
        end: usize,
        cell: CellRef,
    },
    Range {
        start: usize,
        end: usize,
        range: omacell_core::addr::RangeRef,
    },
}

fn collect_ref(expr: &Expr, cursor: u32, out: &mut Option<AnchorTarget>) {
    expr.walk(&mut |node| {
        if node.span.start > cursor || cursor > node.span.end {
            return;
        }
        match &node.kind {
            ExprKind::Cell { cell, .. } => {
                *out = Some(AnchorTarget::Cell {
                    start: node.span.start as usize,
                    end: node.span.end as usize,
                    cell: *cell,
                });
            }
            ExprKind::Range { range, .. } => {
                *out = Some(AnchorTarget::Range {
                    start: node.span.start as usize,
                    end: node.span.end as usize,
                    range: *range,
                });
            }
            _ => {}
        }
    });
}

fn cycle_range(range: &mut omacell_core::addr::RangeRef) -> Result<String, CoreError> {
    if range.whole_col {
        range.start.col_abs = !range.start.col_abs;
        range.end.col_abs = !range.end.col_abs;
        let left = format!(
            "{}{}",
            if range.start.col_abs { "$" } else { "" },
            col_to_letters(range.start.col)?
        );
        let right = format!(
            "{}{}",
            if range.end.col_abs { "$" } else { "" },
            col_to_letters(range.end.col)?
        );
        return Ok(format!("{left}:{right}"));
    }
    if range.whole_row {
        range.start.row_abs = !range.start.row_abs;
        range.end.row_abs = !range.end.row_abs;
        let left = format!(
            "{}{}",
            if range.start.row_abs { "$" } else { "" },
            range.start.row + 1
        );
        let right = format!(
            "{}{}",
            if range.end.row_abs { "$" } else { "" },
            range.end.row + 1
        );
        return Ok(format!("{left}:{right}"));
    }
    cycle_abs(&mut range.start);
    cycle_abs(&mut range.end);
    Ok(format!("{}:{}", a1(&range.start)?, a1(&range.end)?))
}

fn gather_spans(expr: &Expr, out: &mut Vec<(usize, usize)>) {
    expr.walk(&mut |node| {
        if matches!(node.kind, ExprKind::Cell { .. } | ExprKind::Range { .. }) {
            out.push((node.span.start as usize, node.span.end as usize));
        }
    });
}
