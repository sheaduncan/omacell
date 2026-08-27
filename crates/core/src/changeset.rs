//! Changesets: ordered, invertible command lists (spec §8.6, §11.3).

use serde::{Deserialize, Serialize};

use crate::command::{CommandId, Origin};
use crate::error::{CoreError, codes};

/// Identifier of a changeset (opaque, non-empty string; WP-07 assigns them).
///
/// ```
/// use omacell_core::changeset::ChangesetId;
/// let id = ChangesetId::new("cs-1").expect("id");
/// assert_eq!(id.as_str(), "cs-1");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ChangesetId(String);

impl ChangesetId {
    /// Wrap a non-empty id.
    pub fn new(id: impl Into<String>) -> Result<Self, CoreError> {
        let id = id.into();
        if id.is_empty() {
            return Err(CoreError::new(
                codes::CHANGESET_ID,
                "changeset id must be non-empty",
            ));
        }
        Ok(Self(id))
    }

    /// Id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ChangesetId {
    type Error = CoreError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ChangesetId> for String {
    fn from(id: ChangesetId) -> Self {
        id.0
    }
}

/// Lifecycle of a changeset (spec A-6.1).
///
/// ```
/// use omacell_core::changeset::ChangesetStatus;
/// assert_eq!(ChangesetStatus::Proposed, ChangesetStatus::Proposed);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangesetStatus {
    /// Review overlay; not yet applied to the workbook.
    Proposed,
    /// Applied; one undo unit.
    Applied,
    /// Inverse of an applied changeset has been executed.
    Reverted,
}

/// One invocation of a registered command.
///
/// ```
/// use omacell_core::changeset::CommandCall;
/// use omacell_core::command::CommandId;
/// let call = CommandCall {
///     id: CommandId::new("cell.set").expect("id"),
///     args: serde_json::json!({"ref": "A1", "input": "1"}),
/// };
/// assert_eq!(call.id.as_str(), "cell.set");
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandCall {
    /// Command to execute.
    pub id: CommandId,
    /// JSON arguments, validated against the command’s schema (WP-07).
    pub args: serde_json::Value,
}

/// Counts of affected structure, plus a short human summary (spec A-6.1).
///
/// ```
/// use omacell_core::changeset::ChangeSummary;
/// let s = ChangeSummary {
///     cells: 3,
///     rows: 0,
///     columns: 0,
///     sheets: 0,
///     styles: 0,
///     text: "set 3 cells".into(),
/// };
/// assert_eq!(s.cells, 3);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct ChangeSummary {
    /// Cells whose value or formula changed.
    pub cells: u64,
    /// Rows inserted, deleted, hidden, or resized.
    pub rows: u64,
    /// Columns inserted, deleted, hidden, or resized.
    pub columns: u64,
    /// Sheets added, removed, renamed, or reordered.
    pub sheets: u64,
    /// Style records affected.
    pub styles: u64,
    /// Human-readable one-liner for lists and overlays.
    pub text: String,
}

/// Ordered forward commands, computed inverses, origin, and status.
///
/// ```
/// use omacell_core::changeset::{Changeset, ChangesetId, ChangesetStatus, ChangeSummary};
/// use omacell_core::command::Origin;
/// let cs = Changeset {
///     id: ChangesetId::new("cs-1").expect("id"),
///     origin: Origin::User,
///     status: ChangesetStatus::Proposed,
///     forward: vec![],
///     inverse: vec![],
///     summary: ChangeSummary::default(),
/// };
/// assert_eq!(cs.status, ChangesetStatus::Proposed);
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Changeset {
    /// Identity.
    pub id: ChangesetId,
    /// Who produced this changeset.
    pub origin: Origin,
    /// Proposed, applied, or reverted.
    pub status: ChangesetStatus,
    /// Commands to apply.
    pub forward: Vec<CommandCall>,
    /// Inverse commands (same length as `forward`, reverse order when executed).
    pub inverse: Vec<CommandCall>,
    /// Affected-structure summary.
    pub summary: ChangeSummary,
}
