//! Function metadata and JSON catalog.

use omacell_core::eval::{ArrayLift, FnBody, FnDef};
use serde::{Deserialize, Serialize};

/// Envelope schema version for [`functions_json`].
pub const SCHEMA: u32 = 1;

/// Argument kind recorded in metadata (not a type checker).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArgKind {
    /// Numeric.
    Number,
    /// Text.
    Text,
    /// Boolean.
    Logical,
    /// Any scalar/array.
    Any,
    /// Range reference preferred.
    Range,
    /// Array.
    Array,
}

/// Eager vs lazy evaluation strategy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FnStrategy {
    /// Evaluate arguments first.
    Eager,
    /// Special form: implementation chooses which arguments to evaluate.
    Lazy,
}

/// How the function interacts with arrays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArrayBehavior {
    /// No lifting (aggregates, special forms).
    None,
    /// Element-wise lift over arrays.
    LiftAll,
    /// Produces a spilled array.
    ReturnsArray,
}

/// Authoritative function definition. Projects to [`FnDef`].
#[derive(Clone, Copy, Debug)]
pub struct FunctionSpec {
    /// Canonical English name.
    pub name: &'static str,
    /// Alternate spellings.
    pub aliases: &'static [&'static str],
    /// Spec tier (0, 1, 2).
    pub tier: u8,
    /// Category label (`math`, `logical`, …).
    pub category: &'static str,
    /// Argument kinds in declaration order (variadic uses the last kind).
    pub arg_kinds: &'static [ArgKind],
    /// Minimum arity.
    pub min_args: u8,
    /// Maximum arity.
    pub max_args: u8,
    /// Eager or lazy.
    pub strategy: FnStrategy,
    /// Recalculate every pass.
    pub volatile: bool,
    /// Array behaviour.
    pub array: ArrayBehavior,
    /// Async graph node.
    pub async_node: bool,
    /// Human signature.
    pub signature: &'static str,
    /// One-line documentation.
    pub doc: &'static str,
    /// Runtime implementation.
    pub body: FnBody,
}

/// JSON object emitted for one function (stable field set).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionJson {
    /// Canonical name.
    pub name: String,
    /// Aliases.
    pub aliases: Vec<String>,
    /// Tier.
    pub tier: u8,
    /// Category.
    pub category: String,
    /// Argument kinds.
    pub arg_kinds: Vec<ArgKind>,
    /// Min args.
    pub min_args: u8,
    /// Max args.
    pub max_args: u8,
    /// Strategy.
    pub strategy: FnStrategy,
    /// Volatility.
    pub volatile: bool,
    /// Array behaviour.
    pub array: ArrayBehavior,
    /// Async.
    pub async_node: bool,
    /// Signature.
    pub signature: String,
    /// Documentation.
    pub doc: String,
}

/// Catalog envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FunctionsEnvelope {
    /// Schema version.
    pub schema: u32,
    /// Functions sorted by name.
    pub functions: Vec<FunctionJson>,
}

impl FunctionSpec {
    /// Project to the evaluator runtime definition (aliases are extra `FnDef`s).
    #[must_use]
    pub fn to_fn_def(self) -> FnDef {
        spec_to_fn_def(&self)
    }

    /// JSON object for this spec.
    #[must_use]
    pub fn to_json(self) -> FunctionJson {
        FunctionJson {
            name: self.name.to_string(),
            aliases: self.aliases.iter().map(|s| (*s).to_string()).collect(),
            tier: self.tier,
            category: self.category.to_string(),
            arg_kinds: self.arg_kinds.to_vec(),
            min_args: self.min_args,
            max_args: self.max_args,
            strategy: self.strategy,
            volatile: self.volatile,
            array: self.array,
            async_node: self.async_node,
            signature: self.signature.to_string(),
            doc: self.doc.to_string(),
        }
    }
}

/// Project [`FunctionSpec`] onto [`FnDef`].
#[must_use]
pub fn spec_to_fn_def(spec: &FunctionSpec) -> FnDef {
    let array_lift = match spec.array {
        ArrayBehavior::LiftAll => ArrayLift::All,
        ArrayBehavior::None | ArrayBehavior::ReturnsArray => ArrayLift::None,
    };
    FnDef {
        name: spec.name,
        min_args: spec.min_args,
        max_args: spec.max_args,
        volatile: spec.volatile,
        async_node: spec.async_node,
        array_lift,
        body: spec.body,
    }
}

/// Deterministic JSON catalog of every currently registered spec.
#[must_use]
pub fn functions_json() -> String {
    let mut functions: Vec<FunctionJson> = crate::PROBE_SPECS
        .iter()
        .copied()
        .map(FunctionSpec::to_json)
        .collect();
    functions.sort_by(|a, b| a.name.cmp(&b.name));
    let envelope = FunctionsEnvelope {
        schema: SCHEMA,
        functions,
    };
    serde_json::to_string_pretty(&envelope)
        .unwrap_or_else(|_| "{\"schema\":1,\"functions\":[]}".to_string())
}
