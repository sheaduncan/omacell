//! WP-05F probe registrations. WP-05a/b/c replaced every probe.

use omacell_core::eval::FnRegistry;

use crate::metadata::FunctionSpec;

/// No remaining WP-05F probes.
pub const PROBE_SPECS: &[FunctionSpec] = &[];

/// Register remaining probe functions (none after WP-05a/b/c).
pub fn register_probes(_registry: &mut FnRegistry) {}
