//! Changeset store and lifecycle.

use std::io::{self, Write};

use indexmap::IndexMap;
use omacell_core::changeset::{
    ChangeSummary, Changeset, ChangesetId, ChangesetStatus, CommandCall,
};
use omacell_core::command::Origin;
use omacell_core::error::CoreError;
use serde::Serialize;

use crate::error;

/// Maximum retained changesets in one session.
pub const MAX_CHANGESETS: usize = 256;

/// Maximum commands supplied in one changeset proposal.
pub const MAX_CHANGESET_COMMANDS: usize = 1_024;

/// Maximum estimated retained bytes for one changeset.
pub const MAX_CHANGESET_BYTES: usize = 1_048_576;

/// Maximum estimated retained bytes for the complete changeset store.
pub const MAX_CHANGESET_STORE_BYTES: usize = 16 * 1024 * 1024;

/// Maximum inverse, event, or dirty-cell records produced by one bus operation.
pub const MAX_EFFECT_RECORDS: usize = 100_000;

struct Entry {
    public: Changeset,
    inverse: Vec<CommandCall>,
    retained_bytes: usize,
}

/// In-memory changeset store. Proposed records keep inverses private so the
/// frozen [`Changeset`] validator stays satisfied.
pub struct ChangesetStore {
    next: u64,
    entries: IndexMap<String, Entry>,
    retained_bytes: usize,
}

impl Default for ChangesetStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ChangesetStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            next: 1,
            entries: IndexMap::new(),
            retained_bytes: 0,
        }
    }

    /// Number of stored changesets.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the store is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Estimated serialized bytes retained by this store.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    pub(crate) fn ensure_can_propose(&self, forward: &[CommandCall]) -> Result<(), CoreError> {
        if self.entries.len() >= MAX_CHANGESETS {
            return Err(error::changeset_limit(format!(
                "session already retains {MAX_CHANGESETS} changesets"
            )));
        }
        if forward.len() > MAX_CHANGESET_COMMANDS {
            return Err(error::changeset_limit(format!(
                "proposal has {} commands; maximum is {MAX_CHANGESET_COMMANDS}",
                forward.len()
            )));
        }
        let bytes = serialized_len(forward)?;
        if bytes > MAX_CHANGESET_BYTES {
            return Err(error::changeset_limit(format!(
                "proposal command payload is {bytes} bytes; maximum retained changeset size is {MAX_CHANGESET_BYTES}"
            )));
        }
        Ok(())
    }

    pub(crate) fn insert_proposed(
        &mut self,
        origin: Origin,
        forward: Vec<CommandCall>,
        inverse: Vec<CommandCall>,
        summary: ChangeSummary,
    ) -> Result<Changeset, CoreError> {
        self.ensure_can_propose(&forward)?;
        let id = ChangesetId::new(format!("cs-{}", self.next))?;
        let public = Changeset {
            id: id.clone(),
            origin,
            status: ChangesetStatus::Proposed,
            forward,
            inverse: Vec::new(),
            summary,
        };
        public.validate()?;
        let retained_bytes = entry_size(&public.forward, &inverse, &public.summary)?;
        self.ensure_entry_fits(0, retained_bytes)?;
        self.entries.insert(
            id.as_str().to_string(),
            Entry {
                public: public.clone(),
                inverse,
                retained_bytes,
            },
        );
        self.retained_bytes += retained_bytes;
        self.next += 1;
        Ok(public)
    }

    pub(crate) fn replace_proposed(
        &mut self,
        id: &ChangesetId,
        forward: Vec<CommandCall>,
        inverse: Vec<CommandCall>,
        summary: ChangeSummary,
    ) -> Result<Changeset, CoreError> {
        self.ensure_can_retain_forward(&forward)?;
        let entry = self
            .entries
            .get(id.as_str())
            .ok_or_else(|| error::changeset_not_found(id.as_str()))?;
        if entry.public.status != ChangesetStatus::Proposed {
            return Err(error::changeset_state(format!(
                "changeset {} cannot be revised in status {:?}",
                id.as_str(),
                entry.public.status
            )));
        }
        let retained_bytes = entry_size(&forward, &inverse, &summary)?;
        self.ensure_entry_fits(entry.retained_bytes, retained_bytes)?;
        let entry = self
            .entries
            .get_mut(id.as_str())
            .ok_or_else(|| error::changeset_not_found(id.as_str()))?;
        entry.public.forward = forward;
        entry.public.summary = summary;
        entry.inverse = inverse;
        entry.public.validate()?;
        self.retained_bytes = self
            .retained_bytes
            .saturating_sub(entry.retained_bytes)
            .saturating_add(retained_bytes);
        entry.retained_bytes = retained_bytes;
        Ok(entry.public.clone())
    }

    pub(crate) fn remove_proposed(&mut self, id: &ChangesetId) -> Result<Changeset, CoreError> {
        let entry = self
            .entries
            .get(id.as_str())
            .ok_or_else(|| error::changeset_not_found(id.as_str()))?;
        if entry.public.status != ChangesetStatus::Proposed {
            return Err(error::changeset_state(format!(
                "changeset {} cannot be discarded in status {:?}",
                id.as_str(),
                entry.public.status
            )));
        }
        let entry = self
            .entries
            .shift_remove(id.as_str())
            .ok_or_else(|| error::changeset_not_found(id.as_str()))?;
        self.retained_bytes = self.retained_bytes.saturating_sub(entry.retained_bytes);
        Ok(entry.public)
    }

    pub(crate) fn ensure_applied_fits(
        &self,
        id: &ChangesetId,
        inverse: &[CommandCall],
        summary: &ChangeSummary,
    ) -> Result<(), CoreError> {
        let entry = self
            .entries
            .get(id.as_str())
            .ok_or_else(|| error::changeset_not_found(id.as_str()))?;
        let retained_bytes = entry_size(&entry.public.forward, inverse, summary)?;
        self.ensure_entry_fits(entry.retained_bytes, retained_bytes)
    }

    fn ensure_can_retain_forward(&self, forward: &[CommandCall]) -> Result<(), CoreError> {
        if forward.len() > MAX_CHANGESET_COMMANDS {
            return Err(error::changeset_limit(format!(
                "proposal has {} commands; maximum is {MAX_CHANGESET_COMMANDS}",
                forward.len()
            )));
        }
        let bytes = serialized_len(forward)?;
        if bytes > MAX_CHANGESET_BYTES {
            return Err(error::changeset_limit(format!(
                "proposal command payload is {bytes} bytes; maximum retained changeset size is {MAX_CHANGESET_BYTES}"
            )));
        }
        Ok(())
    }

    /// Lookup the public record (proposed inverses stay empty).
    pub fn get(&self, id: &ChangesetId) -> Result<&Changeset, CoreError> {
        self.entries
            .get(id.as_str())
            .map(|entry| &entry.public)
            .ok_or_else(|| error::changeset_not_found(id.as_str()))
    }

    pub(crate) fn inverse_for_revert(&self, id: &ChangesetId) -> Result<&[CommandCall], CoreError> {
        let entry = self
            .entries
            .get(id.as_str())
            .ok_or_else(|| error::changeset_not_found(id.as_str()))?;
        if entry.public.status != ChangesetStatus::Applied {
            return Err(error::changeset_state(format!(
                "changeset {} cannot be reverted in status {:?}",
                id.as_str(),
                entry.public.status
            )));
        }
        Ok(entry.inverse.as_slice())
    }

    pub(crate) fn forward_for_apply(&self, id: &ChangesetId) -> Result<&[CommandCall], CoreError> {
        let entry = self
            .entries
            .get(id.as_str())
            .ok_or_else(|| error::changeset_not_found(id.as_str()))?;
        if entry.public.status != ChangesetStatus::Proposed {
            return Err(error::changeset_state(format!(
                "changeset {} cannot be applied in status {:?}",
                id.as_str(),
                entry.public.status
            )));
        }
        Ok(entry.public.forward.as_slice())
    }

    /// Insertion-order list.
    #[must_use]
    pub fn list(&self) -> Vec<Changeset> {
        self.entries
            .values()
            .map(|entry| entry.public.clone())
            .collect()
    }

    pub(crate) fn mark_applied(
        &mut self,
        id: &ChangesetId,
        inverse: Vec<CommandCall>,
        summary: ChangeSummary,
    ) -> Result<Changeset, CoreError> {
        self.ensure_applied_fits(id, &inverse, &summary)?;
        let entry = self
            .entries
            .get_mut(id.as_str())
            .ok_or_else(|| error::changeset_not_found(id.as_str()))?;
        if entry.public.status != ChangesetStatus::Proposed {
            return Err(error::changeset_state(format!(
                "changeset {} cannot be applied in status {:?}",
                id.as_str(),
                entry.public.status
            )));
        }
        entry.inverse = inverse.clone();
        entry.public.inverse = inverse;
        entry.public.summary = summary;
        entry.public.status = ChangesetStatus::Applied;
        entry.public.validate()?;
        let retained_bytes =
            entry_size(&entry.public.forward, &entry.inverse, &entry.public.summary)?;
        self.retained_bytes = self
            .retained_bytes
            .saturating_sub(entry.retained_bytes)
            .saturating_add(retained_bytes);
        entry.retained_bytes = retained_bytes;
        Ok(entry.public.clone())
    }

    pub(crate) fn mark_reverted(&mut self, id: &ChangesetId) -> Result<Changeset, CoreError> {
        let entry = self
            .entries
            .get_mut(id.as_str())
            .ok_or_else(|| error::changeset_not_found(id.as_str()))?;
        if entry.public.status != ChangesetStatus::Applied {
            return Err(error::changeset_state(format!(
                "changeset {} cannot be reverted in status {:?}",
                id.as_str(),
                entry.public.status
            )));
        }
        entry.public.status = ChangesetStatus::Reverted;
        entry.public.validate()?;
        Ok(entry.public.clone())
    }

    fn ensure_entry_fits(&self, old_bytes: usize, new_bytes: usize) -> Result<(), CoreError> {
        if new_bytes > MAX_CHANGESET_BYTES {
            return Err(error::changeset_limit(format!(
                "changeset retains {new_bytes} bytes; maximum is {MAX_CHANGESET_BYTES}"
            )));
        }
        let projected = self
            .retained_bytes
            .saturating_sub(old_bytes)
            .checked_add(new_bytes)
            .ok_or_else(|| error::changeset_limit("changeset store size overflow"))?;
        if projected > MAX_CHANGESET_STORE_BYTES {
            return Err(error::changeset_limit(format!(
                "changeset store would retain {projected} bytes; maximum is {MAX_CHANGESET_STORE_BYTES}"
            )));
        }
        Ok(())
    }
}

fn entry_size(
    forward: &[CommandCall],
    inverse: &[CommandCall],
    summary: &ChangeSummary,
) -> Result<usize, CoreError> {
    let forward = serialized_len(forward)?;
    let inverse = serialized_len(inverse)?;
    let summary = serialized_len(summary)?;
    // Applied entries expose the inverse publicly and retain a trusted private
    // copy. Reserve both copies at proposal time so lifecycle transitions do
    // not create unbudgeted memory.
    let inverse = inverse
        .checked_mul(2)
        .ok_or_else(|| error::changeset_limit("changeset size overflow"))?;
    forward
        .checked_add(inverse)
        .and_then(|bytes| bytes.checked_add(summary))
        .and_then(|bytes| bytes.checked_add(256))
        .ok_or_else(|| error::changeset_limit("changeset size overflow"))
}

fn serialized_len<T: Serialize + ?Sized>(value: &T) -> Result<usize, CoreError> {
    let mut counter = CountingWriter { bytes: 0 };
    serde_json::to_writer(&mut counter, value)
        .map_err(|err| error::changeset_limit(format!("cannot size changeset: {err}")))?;
    Ok(counter.bytes)
}

struct CountingWriter {
    bytes: usize,
}

impl Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("serialized size overflow"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use omacell_core::changeset::{ChangeSummary, CommandCall};
    use omacell_core::command::{CommandId, Origin};

    use super::{ChangesetStore, MAX_CHANGESET_BYTES, MAX_CHANGESETS};

    #[test]
    fn store_rejects_excessive_count_without_evicting_review_state() {
        let mut store = ChangesetStore::new();
        for _ in 0..MAX_CHANGESETS {
            store
                .insert_proposed(
                    Origin::User,
                    Vec::new(),
                    Vec::new(),
                    ChangeSummary::default(),
                )
                .unwrap();
        }
        let err = store
            .insert_proposed(
                Origin::User,
                Vec::new(),
                Vec::new(),
                ChangeSummary::default(),
            )
            .unwrap_err();
        assert_eq!(err.code, crate::error::codes::CHANGESET_LIMIT);
        assert_eq!(store.len(), MAX_CHANGESETS);
    }

    #[test]
    fn store_rejects_an_oversized_serialized_payload() {
        let mut store = ChangesetStore::new();
        let forward = vec![CommandCall {
            id: CommandId::new("cell.set").unwrap(),
            args: serde_json::json!({"input": "x".repeat(MAX_CHANGESET_BYTES)}),
        }];
        let err = store
            .insert_proposed(Origin::User, forward, Vec::new(), ChangeSummary::default())
            .unwrap_err();
        assert_eq!(err.code, crate::error::codes::CHANGESET_LIMIT);
        assert!(store.is_empty());
    }
}
