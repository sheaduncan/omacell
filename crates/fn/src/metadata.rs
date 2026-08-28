//! Function metadata and JSON catalog.

use omacell_core::eval::{ArrayLift, FnBody, FnDef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Envelope schema version for [`functions_json`].
pub const SCHEMA: u32 = 1;

/// Argument kind recorded in metadata (not a type checker).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FnStrategy {
    /// Evaluate arguments first.
    Eager,
    /// Special form: implementation chooses which arguments to evaluate.
    Lazy,
}

/// How the function interacts with arrays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FunctionsEnvelope {
    /// Schema version.
    pub schema: u32,
    /// Functions sorted by name.
    pub functions: Vec<FunctionJson>,
}

impl FunctionSpec {
    /// Evaluation strategy derived from the runtime body, so metadata and
    /// dispatch cannot disagree.
    #[must_use]
    pub const fn strategy(self) -> FnStrategy {
        match self.body {
            FnBody::Eager(_) => FnStrategy::Eager,
            FnBody::Lazy(_) => FnStrategy::Lazy,
        }
    }

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
            strategy: self.strategy(),
            volatile: self.volatile,
            array: self.array,
            async_node: self.async_node,
            signature: self.signature.to_string(),
            doc: self.doc.to_string(),
        }
    }
}

/// Define an authoritative function specification as data.
///
/// The strategy is intentionally omitted: it is derived from `body`, which
/// prevents the runtime dispatch and generated catalog from drifting apart.
#[macro_export]
macro_rules! define_fn {
    (
        $(#[$meta:meta])*
        $visibility:vis const $identifier:ident = {
            $($field:ident: $value:expr),+ $(,)?
        };
    ) => {
        $(#[$meta])*
        $visibility const $identifier: $crate::FunctionSpec = $crate::FunctionSpec {
            $($field: $value),+
        };
    };
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

/// Every function spec in the catalog, including `ISOMITTED` (not registered).
#[must_use]
pub fn all_specs() -> Vec<FunctionSpec> {
    let mut specs = Vec::new();
    specs.extend_from_slice(crate::PROBE_SPECS);
    specs.extend_from_slice(crate::math::SPECS);
    specs.extend_from_slice(crate::stat::SPECS);
    specs.extend_from_slice(crate::logical::SPECS);
    specs.extend_from_slice(crate::info::SPECS);
    specs.push(crate::info::ISOMITTED_SPEC);
    specs.extend_from_slice(crate::aggregate::SPECS);
    specs.extend_from_slice(crate::TEXT_SPECS);
    specs.extend_from_slice(crate::DATETIME_SPECS);
    specs.extend_from_slice(crate::lookup::SPECS);
    specs.extend_from_slice(crate::array::SPECS);
    for spec in crate::lambda::SPECS {
        if spec.name != "ISOMITTED" {
            specs.push(*spec);
        }
    }
    specs.extend_from_slice(crate::financial::SPECS);
    specs.extend_from_slice(crate::engineering::SPECS);
    specs
}

/// Deterministic JSON catalog of every currently registered spec.
pub fn functions_json() -> Result<String, serde_json::Error> {
    let mut functions: Vec<FunctionJson> =
        all_specs().into_iter().map(FunctionSpec::to_json).collect();
    functions.sort_by(|a, b| a.name.cmp(&b.name));
    let envelope = FunctionsEnvelope {
        schema: SCHEMA,
        functions,
    };
    serde_json::to_string_pretty(&envelope)
}
