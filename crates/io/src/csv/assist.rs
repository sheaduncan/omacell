//! A-4.4 import-assistant hook. This package does not call a model.

use serde::{Deserialize, Serialize};

use super::plan::ImportPlan;
use super::preview::PreviewRows;

/// Payload WP-23 feeds to `ai.import.assist`. Nothing is applied here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportAssistRequest {
    /// Current plan (after sniff / user edits).
    pub plan: ImportPlan,
    /// Preview the assistant should annotate.
    pub preview: PreviewRows,
}

/// Build an assist request. Does not perform I/O or call a provider.
#[must_use]
pub fn import_assist_request(plan: ImportPlan, preview: PreviewRows) -> ImportAssistRequest {
    ImportAssistRequest { plan, preview }
}
