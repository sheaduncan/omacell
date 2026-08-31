//! Defined names at workbook and sheet scope (spec F-1.3).
//!
//! Referents are a range, a constant [`Value`], or formula *text*. Parsing
//! formula text is WP-03.

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::addr::{RangeRef, SheetId, parse_a1_cell, parse_r1c1_cell};
use crate::error::CoreError;
use crate::value::Value;

/// Workbook-wide or sheet-local name.
///
/// ```
/// use omacell_core::names::NameScope;
/// let _ = NameScope::Workbook;
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NameScope {
    /// Visible to every sheet.
    Workbook,
    /// Visible on one sheet (and as `Sheet!Name` from others).
    Sheet(SheetId),
}

/// What a defined name refers to.
///
/// ```
/// use omacell_core::names::NameReferent;
/// use omacell_core::value::Value;
/// let _ = NameReferent::Constant(Value::Number(0.2));
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NameReferent {
    /// A range (possibly 3-D).
    Range(RangeRef),
    /// A constant value (already interned if text/array).
    Constant(Value),
    /// Formula source; WP-03 parses this.
    Formula(String),
}

/// One defined name.
///
/// ```
/// use omacell_core::names::{DefinedName, NameReferent, NameScope};
/// use omacell_core::value::Value;
/// let n = DefinedName {
///     name: "TaxRate".into(),
///     scope: NameScope::Workbook,
///     referent: NameReferent::Constant(Value::Number(0.2)),
///     comment: None,
/// };
/// assert_eq!(n.name, "TaxRate");
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DefinedName {
    /// Display name (original case).
    pub name: String,
    /// Workbook or sheet scope.
    pub scope: NameScope,
    /// Range, constant, or formula text.
    pub referent: NameReferent,
    /// Optional comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// Validate Excel-ish defined-name syntax.
pub fn validate_defined_name(name: &str) -> Result<(), CoreError> {
    if name.is_empty() || name.chars().count() > 255 {
        return Err(CoreError::name_defined(
            "defined names are 1–255 characters and cannot be empty",
        ));
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(CoreError::name_defined("empty defined name"));
    };
    if !(first.is_alphabetic() || first == '_' || first == '\\') {
        return Err(CoreError::name_defined(format!(
            "defined name {name:?} must start with a letter, '_', or '\\\\'"
        )));
    }
    if !chars.all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '?') {
        return Err(CoreError::name_defined(format!(
            "defined name {name:?} contains a character that is not letter, digit, '_', '.', or '?'"
        )));
    }
    let upper = name.to_ascii_uppercase();
    if upper == "R" || upper == "C" {
        return Err(CoreError::name_defined(
            "R and C are reserved row/column specifiers",
        ));
    }
    if upper == "TRUE" || upper == "FALSE" {
        return Err(CoreError::name_defined(
            "TRUE and FALSE are reserved boolean literals",
        ));
    }
    if parse_a1_cell(name).is_ok() {
        return Err(CoreError::name_defined(format!(
            "defined name {name:?} looks like an A1 cell"
        )));
    }
    if parse_r1c1_cell(name, 0, 0).is_ok() {
        return Err(CoreError::name_defined(format!(
            "defined name {name:?} looks like an R1C1 cell"
        )));
    }
    Ok(())
}

fn key(scope: NameScope, name: &str) -> (NameScope, String) {
    (scope, name.to_lowercase())
}

/// Registry of defined names. Iteration is sorted by (scope, lowercase name).
///
/// ```
/// use omacell_core::names::{DefinedName, NameReferent, NameRegistry, NameScope};
/// use omacell_core::value::Value;
/// let mut r = NameRegistry::new();
/// r.insert(DefinedName {
///     name: "TaxRate".into(),
///     scope: NameScope::Workbook,
///     referent: NameReferent::Constant(Value::Number(0.2)),
///     comment: None,
/// }).unwrap();
/// assert!(r.get(NameScope::Workbook, "taxrate").is_some());
/// ```
#[derive(Clone, Debug, Default)]
pub struct NameRegistry {
    by_key: FxHashMap<(NameScope, String), DefinedName>,
}

impl NameRegistry {
    /// Empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert. Duplicate (scope, name) ignoring case is an error.
    pub fn insert(&mut self, name: DefinedName) -> Result<(), CoreError> {
        validate_defined_name(&name.name)?;
        let k = key(name.scope, &name.name);
        if self.by_key.contains_key(&k) {
            return Err(CoreError::name_defined(format!(
                "defined name {:?} already exists in this scope",
                name.name
            )));
        }
        self.by_key.insert(k, name);
        Ok(())
    }

    /// Replace or insert. Returns the previous value.
    pub fn upsert(&mut self, name: DefinedName) -> Result<Option<DefinedName>, CoreError> {
        validate_defined_name(&name.name)?;
        let k = key(name.scope, &name.name);
        Ok(self.by_key.insert(k, name))
    }

    /// Remove by scope and name (case-insensitive).
    pub fn remove(&mut self, scope: NameScope, name: &str) -> Option<DefinedName> {
        self.by_key.remove(&key(scope, name))
    }

    /// Lookup (case-insensitive).
    #[must_use]
    pub fn get(&self, scope: NameScope, name: &str) -> Option<&DefinedName> {
        self.by_key.get(&key(scope, name))
    }

    /// Resolve a name: sheet scope first, then workbook.
    #[must_use]
    pub fn resolve(&self, sheet: SheetId, name: &str) -> Option<&DefinedName> {
        self.get(NameScope::Sheet(sheet), name)
            .or_else(|| self.get(NameScope::Workbook, name))
    }

    /// Sorted iterator.
    pub fn iter(&self) -> impl Iterator<Item = &DefinedName> {
        let mut items: Vec<&DefinedName> = self.by_key.values().collect();
        items.sort_by(|a, b| {
            scope_ord(a.scope)
                .cmp(&scope_ord(b.scope))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        items.into_iter()
    }

    /// Number of names.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}

fn scope_ord(scope: NameScope) -> (u8, u32) {
    match scope {
        NameScope::Workbook => (0, 0),
        NameScope::Sheet(id) => (1, id.index()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_cell_like_names() {
        assert!(validate_defined_name("A1").is_err());
        assert!(validate_defined_name("R1C1").is_err());
        assert!(validate_defined_name("R").is_err());
        assert!(validate_defined_name("Revenue").is_ok());
    }

    #[test]
    fn length_limit_counts_unicode_characters() {
        assert!(validate_defined_name(&"é".repeat(255)).is_ok());
        assert!(validate_defined_name(&"é".repeat(256)).is_err());
    }

    #[test]
    fn case_insensitive_unique() {
        let mut r = NameRegistry::new();
        r.insert(DefinedName {
            name: "TaxRate".into(),
            scope: NameScope::Workbook,
            referent: NameReferent::Constant(Value::Number(0.2)),
            comment: None,
        })
        .unwrap();
        assert!(
            r.insert(DefinedName {
                name: "taxrate".into(),
                scope: NameScope::Workbook,
                referent: NameReferent::Constant(Value::Number(0.1)),
                comment: None,
            })
            .is_err()
        );
        assert!(
            r.insert(DefinedName {
                name: "TaxRate".into(),
                scope: NameScope::Sheet(SheetId::new(0)),
                referent: NameReferent::Constant(Value::Number(0.3)),
                comment: None,
            })
            .is_ok()
        );
    }
}
