//! Workbook-scoped interners (spec §11.3).
//!
//! Strings are a shared-string table: stable ids while live, refcounts, and
//! optional rich-text runs. Styles are interned by value. Formula *source*
//! is interned as [`FormulaId`]; parsing the AST is WP-03.

use std::mem::size_of;
use std::sync::Arc;

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, codes};
use crate::limits::MAX_FORMULA_LEN;
use crate::style::{Font, Style, StyleId};
use crate::value::{Array2D, ArrayId, StrId, Value};

/// Handle to interned formula source text. The AST is owned by WP-03.
///
/// ```
/// use omacell_core::intern::FormulaId;
/// let id = FormulaId::new(3);
/// assert_eq!(id.index(), 3);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FormulaId(u32);

impl FormulaId {
    /// Wrap an intern index.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Intern index.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// One rich-text run (F-2.5). Offsets are UTF-8 bytes into the cell text.
///
/// ```
/// use omacell_core::intern::RichTextRun;
/// use omacell_core::style::Font;
/// let run = RichTextRun { start: 0, len: 3, font: Font::default() };
/// assert_eq!(run.start, 0);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RichTextRun {
    /// Byte offset of the run.
    pub start: u32,
    /// Byte length of the run.
    pub len: u32,
    /// Font for this run.
    pub font: Font,
}

/// Spill / literal array payload behind [`ArrayId`].
///
/// ```
/// use omacell_core::intern::ArrayPayload;
/// use omacell_core::value::{Array2D, Value};
/// let p = ArrayPayload::new(Array2D::new(1, 2).unwrap(), vec![Value::Number(1.0), Value::Number(2.0)]).unwrap();
/// assert_eq!(p.len(), 2);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct ArrayPayload {
    /// Shape.
    pub shape: Array2D,
    /// Row-major values. Length equals `shape.len()`.
    pub values: Arc<[Value]>,
}

impl ArrayPayload {
    /// Construct a payload whose length matches `shape`.
    pub fn new(shape: Array2D, values: Vec<Value>) -> Result<Self, CoreError> {
        if values.len() as u32 != shape.len() {
            return Err(CoreError::new(
                codes::ARRAY_SHAPE,
                format!(
                    "array payload length {} does not match shape {}×{}",
                    values.len(),
                    shape.rows,
                    shape.cols
                ),
            ));
        }
        Ok(Self {
            shape,
            values: values.into(),
        })
    }

    /// Cell count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether the payload is empty (never for a value produced by [`Self::new`]).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

#[derive(Clone, Debug)]
struct StringEntry {
    text: Arc<str>,
    rich: Option<Arc<[RichTextRun]>>,
    refs: u32,
}

#[derive(Clone, Debug)]
struct StyleEntry {
    style: Style,
    refs: u32,
}

#[derive(Clone, Debug)]
struct ArrayEntry {
    payload: ArrayPayload,
    refs: u32,
}

#[derive(Clone, Debug)]
struct FormulaEntry {
    source: Arc<str>,
    refs: u32,
}

/// Shared-string table (plain and rich).
#[derive(Clone, Debug, Default)]
pub struct StringInterner {
    entries: Vec<Option<StringEntry>>,
    by_plain: FxHashMap<Arc<str>, StrId>,
    free: Vec<u32>,
}

impl StringInterner {
    /// Intern a plain string, incrementing its refcount.
    pub fn intern(&mut self, text: &str) -> StrId {
        if let Some(&id) = self.by_plain.get(text) {
            self.add_ref(id);
            return id;
        }
        let text: Arc<str> = text.into();
        let id = self.alloc(StringEntry {
            text: Arc::clone(&text),
            rich: None,
            refs: 1,
        });
        self.by_plain.insert(text, id);
        id
    }

    /// Intern text with rich-text runs. Runs are part of identity.
    pub fn intern_rich(&mut self, text: &str, runs: Vec<RichTextRun>) -> StrId {
        if runs.is_empty() {
            return self.intern(text);
        }
        for (i, slot) in self.entries.iter_mut().enumerate() {
            if let Some(entry) = slot
                && entry.text.as_ref() == text
                && entry.rich.as_deref() == Some(runs.as_slice())
            {
                entry.refs = entry.refs.saturating_add(1);
                return StrId::new(i as u32);
            }
        }
        self.alloc(StringEntry {
            text: text.into(),
            rich: Some(runs.into()),
            refs: 1,
        })
    }

    fn alloc(&mut self, entry: StringEntry) -> StrId {
        if let Some(i) = self.free.pop() {
            self.entries[i as usize] = Some(entry);
            StrId::new(i)
        } else {
            let i = self.entries.len() as u32;
            self.entries.push(Some(entry));
            StrId::new(i)
        }
    }

    /// Increment the refcount. Unknown ids are ignored.
    pub fn add_ref(&mut self, id: StrId) {
        if let Some(Some(entry)) = self.entries.get_mut(id.index() as usize) {
            entry.refs = entry.refs.saturating_add(1);
        }
    }

    /// Decrement the refcount and recycle the id at zero.
    pub fn release(&mut self, id: StrId) {
        let i = id.index() as usize;
        let recycle = match self.entries.get_mut(i) {
            Some(Some(entry)) => {
                entry.refs = entry.refs.saturating_sub(1);
                entry.refs == 0
            }
            _ => false,
        };
        if recycle && let Some(entry) = self.entries[i].take() {
            if entry.rich.is_none() {
                self.by_plain.remove(&entry.text);
            }
            self.free.push(id.index());
        }
    }

    /// Borrow the interned text.
    #[must_use]
    pub fn get(&self, id: StrId) -> Option<&str> {
        self.entries
            .get(id.index() as usize)
            .and_then(|e| e.as_ref().map(|e| e.text.as_ref()))
    }

    /// Borrow rich-text runs, if any.
    #[must_use]
    pub fn get_rich(&self, id: StrId) -> Option<&[RichTextRun]> {
        self.entries
            .get(id.index() as usize)
            .and_then(|e| e.as_ref().and_then(|e| e.rich.as_deref()))
    }

    /// Number of live (refcount > 0) strings.
    #[must_use]
    pub fn live_len(&self) -> usize {
        self.entries.len() - self.free.len()
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        let mut n = self.entries.capacity() * size_of::<Option<StringEntry>>();
        n += self.free.capacity() * size_of::<u32>();
        n += hashmap_bytes(self.by_plain.capacity(), size_of::<(Arc<str>, StrId)>());
        for e in self.entries.iter().flatten() {
            n += e.text.len();
            if let Some(rich) = &e.rich {
                n += rich.len() * size_of::<RichTextRun>();
            }
        }
        n
    }
}

/// Style table, deduplicated by value.
#[derive(Clone, Debug)]
pub struct StyleInterner {
    entries: Vec<StyleEntry>,
    by_value: FxHashMap<Style, StyleId>,
}

impl Default for StyleInterner {
    fn default() -> Self {
        let default = Style::default();
        let mut by_value = FxHashMap::default();
        by_value.insert(default.clone(), StyleId::DEFAULT);
        Self {
            entries: vec![StyleEntry {
                style: default,
                refs: u32::MAX / 4,
            }],
            by_value,
        }
    }
}

impl StyleInterner {
    /// Intern a style. [`Style::default`] is always [`StyleId::DEFAULT`].
    pub fn intern(&mut self, style: Style) -> StyleId {
        if let Some(&id) = self.by_value.get(&style) {
            if id != StyleId::DEFAULT {
                self.entries[id.index() as usize].refs =
                    self.entries[id.index() as usize].refs.saturating_add(1);
            }
            return id;
        }
        let id = StyleId::new(self.entries.len() as u32);
        self.by_value.insert(style.clone(), id);
        self.entries.push(StyleEntry { style, refs: 1 });
        id
    }

    /// Increment the refcount. Default style is ignored.
    pub fn add_ref(&mut self, id: StyleId) {
        if id == StyleId::DEFAULT {
            return;
        }
        if let Some(entry) = self.entries.get_mut(id.index() as usize) {
            entry.refs = entry.refs.saturating_add(1);
        }
    }

    /// Decrement the refcount. Zero-ref styles stay in the table (stable ids
    /// for the session) but can be reused by value on the next intern.
    pub fn release(&mut self, id: StyleId) {
        if id == StyleId::DEFAULT {
            return;
        }
        if let Some(entry) = self.entries.get_mut(id.index() as usize) {
            entry.refs = entry.refs.saturating_sub(1);
        }
    }

    /// Borrow a style record.
    #[must_use]
    pub fn get(&self, id: StyleId) -> Option<&Style> {
        self.entries.get(id.index() as usize).map(|e| &e.style)
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<StyleEntry>()
            + hashmap_bytes(self.by_value.capacity(), size_of::<(Style, StyleId)>())
    }
}

/// Array payload table.
#[derive(Clone, Debug, Default)]
pub struct ArrayInterner {
    entries: Vec<Option<ArrayEntry>>,
    free: Vec<u32>,
}

impl ArrayInterner {
    /// Intern a payload.
    pub fn intern(&mut self, payload: ArrayPayload) -> ArrayId {
        for (i, slot) in self.entries.iter_mut().enumerate() {
            if let Some(entry) = slot
                && entry.payload.shape == payload.shape
                && entry.payload.values.as_ref() == payload.values.as_ref()
            {
                entry.refs = entry.refs.saturating_add(1);
                return ArrayId::new(i as u32);
            }
        }
        let entry = ArrayEntry { payload, refs: 1 };
        if let Some(i) = self.free.pop() {
            self.entries[i as usize] = Some(entry);
            ArrayId::new(i)
        } else {
            let i = self.entries.len() as u32;
            self.entries.push(Some(entry));
            ArrayId::new(i)
        }
    }

    /// Increment the refcount.
    pub fn add_ref(&mut self, id: ArrayId) {
        if let Some(Some(entry)) = self.entries.get_mut(id.index() as usize) {
            entry.refs = entry.refs.saturating_add(1);
        }
    }

    /// Decrement and recycle at zero.
    pub fn release(&mut self, id: ArrayId) {
        let i = id.index() as usize;
        let recycle = match self.entries.get_mut(i) {
            Some(Some(entry)) => {
                entry.refs = entry.refs.saturating_sub(1);
                entry.refs == 0
            }
            _ => false,
        };
        if recycle {
            self.entries[i] = None;
            self.free.push(id.index());
        }
    }

    /// Borrow a payload.
    #[must_use]
    pub fn get(&self, id: ArrayId) -> Option<&ArrayPayload> {
        self.entries
            .get(id.index() as usize)
            .and_then(|e| e.as_ref().map(|e| &e.payload))
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        let mut n = self.entries.capacity() * size_of::<Option<ArrayEntry>>();
        n += self.free.capacity() * 4;
        for e in self.entries.iter().flatten() {
            n += e.payload.values.len() * size_of::<Value>();
        }
        n
    }
}

/// Formula source table. WP-03 attaches an AST to the same [`FormulaId`].
#[derive(Clone, Debug, Default)]
pub struct FormulaInterner {
    entries: Vec<Option<FormulaEntry>>,
    by_source: FxHashMap<Arc<str>, FormulaId>,
    free: Vec<u32>,
}

impl FormulaInterner {
    /// Intern formula source. Enforces [`MAX_FORMULA_LEN`].
    pub fn intern(&mut self, source: &str) -> Result<FormulaId, CoreError> {
        if source.len() > MAX_FORMULA_LEN {
            return Err(CoreError::formula_len(format!(
                "formula is {} bytes; max is {MAX_FORMULA_LEN}",
                source.len()
            )));
        }
        if let Some(&id) = self.by_source.get(source) {
            self.add_ref(id);
            return Ok(id);
        }
        let source: Arc<str> = source.into();
        let entry = FormulaEntry {
            source: Arc::clone(&source),
            refs: 1,
        };
        let id = if let Some(i) = self.free.pop() {
            self.entries[i as usize] = Some(entry);
            FormulaId::new(i)
        } else {
            let i = self.entries.len() as u32;
            self.entries.push(Some(entry));
            FormulaId::new(i)
        };
        self.by_source.insert(source, id);
        Ok(id)
    }

    /// Increment the refcount.
    pub fn add_ref(&mut self, id: FormulaId) {
        if let Some(Some(entry)) = self.entries.get_mut(id.index() as usize) {
            entry.refs = entry.refs.saturating_add(1);
        }
    }

    /// Decrement and recycle at zero.
    pub fn release(&mut self, id: FormulaId) {
        let i = id.index() as usize;
        let recycle = match self.entries.get_mut(i) {
            Some(Some(entry)) => {
                entry.refs = entry.refs.saturating_sub(1);
                entry.refs == 0
            }
            _ => false,
        };
        if recycle && let Some(entry) = self.entries[i].take() {
            self.by_source.remove(&entry.source);
            self.free.push(id.index());
        }
    }

    /// Borrow formula source.
    #[must_use]
    pub fn get(&self, id: FormulaId) -> Option<&str> {
        self.entries
            .get(id.index() as usize)
            .and_then(|e| e.as_ref().map(|e| e.source.as_ref()))
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        let mut n = self.entries.capacity() * size_of::<Option<FormulaEntry>>();
        n += self.free.capacity() * 4;
        n += hashmap_bytes(
            self.by_source.capacity(),
            size_of::<(Arc<str>, FormulaId)>(),
        );
        for e in self.entries.iter().flatten() {
            n += e.source.len();
        }
        n
    }
}

/// All interners owned by a workbook.
///
/// ```
/// use omacell_core::intern::Interners;
/// let mut i = Interners::new();
/// let id = i.strings.intern("hello");
/// assert_eq!(i.strings.get(id), Some("hello"));
/// ```
#[derive(Clone, Debug, Default)]
pub struct Interners {
    /// Shared-string table.
    pub strings: StringInterner,
    /// Style table. Index 0 is [`Style::default`].
    pub styles: StyleInterner,
    /// Array payloads.
    pub arrays: ArrayInterner,
    /// Formula source (not AST).
    pub formulas: FormulaInterner,
}

impl Interners {
    /// Empty interners with the default style at id 0.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Estimated heap used by intern tables.
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        self.strings.heap_bytes()
            + self.styles.heap_bytes()
            + self.arrays.heap_bytes()
            + self.formulas.heap_bytes()
    }
}

pub(crate) fn hashmap_bytes(capacity: usize, slot: usize) -> usize {
    capacity.saturating_mul(slot.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_intern_stable_and_refcounted() {
        let mut s = StringInterner::default();
        let a = s.intern("hello");
        let b = s.intern("hello");
        assert_eq!(a, b);
        s.release(a);
        assert_eq!(s.get(b), Some("hello"));
        s.release(b);
        assert_eq!(s.get(a), None);
        let c = s.intern("hello");
        assert_eq!(s.get(c), Some("hello"));
    }

    #[test]
    fn style_default_is_zero() {
        let mut s = StyleInterner::default();
        let id = s.intern(Style::default());
        assert_eq!(id, StyleId::DEFAULT);
        let mut other = Style::default();
        other.font.bold = true;
        let b = s.intern(other.clone());
        assert_ne!(b, StyleId::DEFAULT);
        let b2 = s.intern(other);
        assert_eq!(b, b2);
    }

    #[test]
    fn formula_enforces_max_len() {
        let mut f = FormulaInterner::default();
        let too = "a".repeat(MAX_FORMULA_LEN + 1);
        assert_eq!(f.intern(&too).unwrap_err().code, codes::FORMULA_LEN);
        let id = f.intern("=A1").unwrap();
        assert_eq!(f.get(id), Some("=A1"));
    }
}
