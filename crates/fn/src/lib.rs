//! Excel-compatible function library and registry for Omacell.
//!
//! Depends on `omacell-core` only. WP-05F owns metadata, dispatch projection,
//! probe registrations, and the shared corpus runner. WP-05a fills math, stat,
//! logical, information, and criteria aggregation; WP-05b fills text and
//! date/time; WP-05c replaces remaining probes.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod aggregate;
mod common;
mod corpus;
mod datetime;
mod info;
mod logical;
mod math;
mod metadata;
mod probes;
mod stat;
mod text;
mod util;

pub use aggregate::register_aggregate;
pub use corpus::{CorpusRow, run_corpus_file};
pub use datetime::{DATETIME_SPECS, register_datetime};
pub use info::register_info;
pub use logical::register_logical;
pub use math::register_math;
pub use metadata::{
    ArgKind, ArrayBehavior, FnStrategy, FunctionJson, FunctionSpec, FunctionsEnvelope, SCHEMA,
    all_specs, functions_json, spec_to_fn_def,
};
pub use probes::{PROBE_SPECS, register_probes};
pub use stat::register_stat;
pub use text::{TEXT_SPECS, register_text};

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
}
