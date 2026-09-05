//! Excel-compatible function library and registry for Omacell.
//!
//! Depends on `omacell-core` only. WP-05F owns metadata, dispatch projection,
//! and the shared corpus runner. WP-05a fills math, stat, logical, information,
//! and criteria aggregation; WP-05b fills text and date/time; WP-05c fills
//! lookup, dynamic arrays, lambda helpers, financial, and engineering.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod aggregate;
mod args;
mod array;
mod common;
mod corpus;
mod datetime;
mod engineering;
mod financial;
mod info;
mod lambda;
mod logical;
mod lookup;
mod math;
mod metadata;
mod probes;
mod stat;
mod text;
mod util;

pub use aggregate::register_aggregate;
pub use array::SPECS as ARRAY_SPECS;
pub use corpus::{CorpusRow, run_corpus_file};
pub use datetime::{DATETIME_SPECS, register_datetime};
pub use engineering::SPECS as ENGINEERING_SPECS;
pub use financial::SPECS as FINANCIAL_SPECS;
pub use info::register_info;
pub use lambda::SPECS as LAMBDA_SPECS;
pub use logical::register_logical;
pub use lookup::SPECS as LOOKUP_SPECS;
pub use math::register_math;
pub use metadata::{
    ArgKind, ArrayBehavior, FnStrategy, FunctionJson, FunctionSpec, FunctionsEnvelope, SCHEMA,
    all_specs, functions_json, spec_to_fn_def,
};
pub use probes::{PROBE_SPECS, register_probes};
pub use stat::register_stat;
pub use text::{TEXT_SPECS, register_text};

#[cfg(feature = "fuzzing")]
pub use args::parse_address;
#[cfg(feature = "fuzzing")]
pub use common::parse_criteria;

use omacell_core::eval::FnRegistry;

/// Register every currently shipped function.
pub fn register_all(registry: &mut FnRegistry) {
    register_probes(registry);
    register_math(registry);
    register_stat(registry);
    register_logical(registry);
    register_info(registry);
    register_aggregate(registry);
    register_text(registry);
    register_datetime(registry);
    lookup::register_lookup(registry);
    array::register_array(registry);
    lambda::register_lambda(registry);
    financial::register_financial(registry);
    engineering::register_engineering(registry);
}
