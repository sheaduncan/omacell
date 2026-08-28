//! Excel-compatible function library and registry for Omacell.
//!
//! Depends on `omacell-core` only. WP-05F owns metadata, dispatch projection,
//! probe registrations, and the shared corpus runner. WP-05a/b/c replace the
//! probes with the Tier-0 library.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod corpus;
mod metadata;
mod probes;

pub use corpus::{CorpusRow, run_corpus_file};
pub use metadata::{
    ArgKind, ArrayBehavior, FnStrategy, FunctionJson, FunctionSpec, FunctionsEnvelope, SCHEMA,
    functions_json, spec_to_fn_def,
};
pub use probes::{PROBE_SPECS, register_probes};

use omacell_core::eval::FnRegistry;

/// Register every currently shipped function (probes until WP-05a/b/c).
pub fn register_all(registry: &mut FnRegistry) {
    register_probes(registry);
}
