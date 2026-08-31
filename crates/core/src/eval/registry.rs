//! Function registry consumed by the evaluator. WP-05 fills the library.

use std::fmt;
use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::error::ErrorKind;
use crate::formula::Expr;

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

/// Eager vs lazy argument evaluation. `LET`/`LAMBDA`/`ISOMITTED` stay
/// evaluator language constructs and are not registered here.
#[derive(Clone, Copy, Debug)]
pub enum FnBody {
    /// Arguments are evaluated before the implementation runs.
    Eager(fn(&mut EvalCtx<'_>, &[ArgVal]) -> RuntimeValue),
    /// Implementation receives unevaluated argument expressions.
    Lazy(fn(&mut EvalCtx<'_>, &[Option<Expr>]) -> RuntimeValue),
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
    /// Array-lifting behaviour (eager functions only).
    pub array_lift: ArrayLift,
    /// Implementation. Must not panic on any input.
    pub body: FnBody,
}

impl FnDef {
    /// Eager function (evaluated arguments).
    pub const fn eager(
        name: &'static str,
        min_args: u8,
        max_args: u8,
        volatile: bool,
        async_node: bool,
        array_lift: ArrayLift,
        eval: fn(&mut EvalCtx<'_>, &[ArgVal]) -> RuntimeValue,
    ) -> Self {
        Self {
            name,
            min_args,
            max_args,
            volatile,
            async_node,
            array_lift,
            body: FnBody::Eager(eval),
        }
    }

    /// Lazy / special-form function (unevaluated argument expressions).
    pub const fn lazy(
        name: &'static str,
        min_args: u8,
        max_args: u8,
        volatile: bool,
        eval: fn(&mut EvalCtx<'_>, &[Option<Expr>]) -> RuntimeValue,
    ) -> Self {
        Self {
            name,
            min_args,
            max_args,
            volatile,
            async_node: false,
            array_lift: ArrayLift::None,
            body: FnBody::Lazy(eval),
        }
    }
}

/// Host-provided custom function (Lua, WP-20).
pub trait DynamicFnBody: Send + Sync {
    /// Evaluate with already-computed arguments.
    fn eval(&self, args: &[ArgVal]) -> RuntimeValue;
}

/// One dynamically registered function.
#[derive(Clone)]
pub struct DynamicFn {
    /// Canonical name (`USER.DOUBLE`).
    pub name: String,
    /// Minimum argument count.
    pub min_args: u8,
    /// Maximum argument count.
    pub max_args: u8,
    /// Recalculate every pass.
    pub volatile: bool,
    /// Array-lifting behaviour.
    pub array_lift: ArrayLift,
    /// Implementation.
    pub body: Arc<dyn DynamicFnBody>,
}

impl fmt::Debug for DynamicFn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DynamicFn")
            .field("name", &self.name)
            .field("min_args", &self.min_args)
            .field("max_args", &self.max_args)
            .field("volatile", &self.volatile)
            .field("array_lift", &self.array_lift)
            .finish_non_exhaustive()
    }
}

/// Case-insensitive function table. Unknown names evaluate to `#NAME?`.
///
/// ```
/// use omacell_core::eval::FnRegistry;
/// let r = FnRegistry::new();
/// assert!(r.lookup("SUM").is_none());
/// ```
#[derive(Clone, Default)]
pub struct FnRegistry {
    by_upper: FxHashMap<String, FnDef>,
    dynamic: FxHashMap<String, Arc<DynamicFn>>,
}

impl fmt::Debug for FnRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FnRegistry")
            .field("builtins", &self.by_upper.len())
            .field("dynamic", &self.dynamic.len())
            .finish()
    }
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

    /// Insert or replace a dynamic (Lua) function. Names are case-insensitive.
    pub fn register_dynamic(&mut self, def: DynamicFn) {
        let key = def.name.to_ascii_uppercase();
        self.dynamic.insert(key, Arc::new(def));
    }

    /// Lookup a built-in by worksheet spelling.
    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<&FnDef> {
        self.by_upper.get(&name.to_ascii_uppercase())
    }

    /// Lookup a dynamic function by worksheet spelling.
    #[must_use]
    pub fn lookup_dynamic(&self, name: &str) -> Option<&DynamicFn> {
        self.dynamic
            .get(&name.to_ascii_uppercase())
            .map(Arc::as_ref)
    }

    /// Sorted iterator of built-ins (deterministic).
    pub fn iter(&self) -> impl Iterator<Item = &FnDef> {
        let mut keys: Vec<&String> = self.by_upper.keys().collect();
        keys.sort_unstable();
        keys.into_iter().filter_map(|k| self.by_upper.get(k))
    }

    /// Sorted iterator of dynamic functions.
    pub fn iter_dynamic(&self) -> impl Iterator<Item = &DynamicFn> {
        let mut keys: Vec<&String> = self.dynamic.keys().collect();
        keys.sort_unstable();
        keys.into_iter()
            .filter_map(|k| self.dynamic.get(k).map(Arc::as_ref))
    }

    /// Number of registered functions (built-in plus dynamic).
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_upper.len() + self.dynamic.len()
    }

    /// Whether the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_upper.is_empty() && self.dynamic.is_empty()
    }
}

/// `#NAME?` for an unknown function.
#[must_use]
pub fn name_error() -> RuntimeValue {
    RuntimeValue::error(ErrorKind::Name)
}
