//! Function registry consumed by the evaluator. WP-05 fills the library.

use rustc_hash::FxHashMap;

use crate::error::ErrorKind;

use super::{ArgVal, EvalCtx, RuntimeValue};

/// How a function wants its arguments.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ArrayLift {
    /// Pass references through (aggregates like `SUM`).
    #[default]
    None,
    /// Lift the function over arrays / ranges element-wise (`ABS`).
    All,
}

/// One registered function.
#[derive(Clone, Copy, Debug)]
pub struct FnDef {
    /// Canonical English name (uppercase).
    pub name: &'static str,
    /// Minimum argument count (omitted args still count).
    pub min_args: u8,
    /// Maximum argument count (inclusive).
    pub max_args: u8,
    /// Recalculate every pass (F-3.6).
    pub volatile: bool,
    /// Asynchronous graph node (A-3.3).
    pub async_node: bool,
    /// Array-lifting behaviour.
    pub array_lift: ArrayLift,
    /// Implementation. Must not panic on any input.
    pub eval: fn(&mut EvalCtx<'_>, &[ArgVal]) -> RuntimeValue,
}

/// Case-insensitive function table. Unknown names evaluate to `#NAME?`.
///
/// ```
/// use omacell_core::eval::FnRegistry;
/// let r = FnRegistry::new();
/// assert!(r.lookup("SUM").is_none());
/// ```
#[derive(Clone, Debug, Default)]
pub struct FnRegistry {
    by_upper: FxHashMap<String, FnDef>,
}

impl FnRegistry {
    /// Empty registry (production default; WP-05 registers the library).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace a definition. Names are matched case-insensitively.
    pub fn register(&mut self, def: FnDef) {
        self.by_upper.insert(def.name.to_ascii_uppercase(), def);
    }

    /// Lookup by worksheet spelling.
    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<&FnDef> {
        self.by_upper.get(&name.to_ascii_uppercase())
    }

    /// Sorted iterator (deterministic).
    pub fn iter(&self) -> impl Iterator<Item = &FnDef> {
        let mut keys: Vec<&String> = self.by_upper.keys().collect();
        keys.sort_unstable();
        keys.into_iter().filter_map(|k| self.by_upper.get(k))
    }

    /// Number of registered functions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_upper.len()
    }

    /// Whether the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_upper.is_empty()
    }
}

/// `#NAME?` for an unknown function.
#[must_use]
pub fn name_error() -> RuntimeValue {
    RuntimeValue::error(ErrorKind::Name)
}
