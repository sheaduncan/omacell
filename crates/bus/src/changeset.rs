//! Changeset store and lifecycle.

use indexmap::IndexMap;
use omacell_core::changeset::{
    ChangeSummary, Changeset, ChangesetId, ChangesetStatus, CommandCall,
};
use omacell_core::command::Origin;
use omacell_core::error::CoreError;

use crate::error;

struct Entry {
    public: Changeset,
    inverse: Vec<CommandCall>,
}

/// In-memory changeset store. Proposed records keep inverses private so the
/// frozen [`Changeset`] validator stays satisfied.
pub struct ChangesetStore {
    next: u64,
    entries: IndexMap<String, Entry>,
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

    pub(crate) fn insert_proposed(
        &mut self,
        origin: Origin,
        forward: Vec<CommandCall>,
        inverse: Vec<CommandCall>,
        summary: ChangeSummary,
    ) -> Result<Changeset, CoreError> {
        let id = ChangesetId::new(format!("cs-{}", self.next))?;
        self.next += 1;
        let public = Changeset {
            id: id.clone(),
            origin,
            status: ChangesetStatus::Proposed,
            forward,
            inverse: Vec::new(),
            summary,
        };
        public.validate()?;
        self.entries.insert(
            id.as_str().to_string(),
            Entry {
                public: public.clone(),
                inverse,
            },
        );
        Ok(public)
    }

    /// Lookup the public record (proposed inverses stay empty).
    pub fn get(&self, id: &ChangesetId) -> Result<&Changeset, CoreError> {
        self.entries
            .get(id.as_str())
            .map(|entry| &entry.public)
            .ok_or_else(|| error::changeset_not_found(id.as_str()))
    }

    pub(crate) fn inverse(&self, id: &ChangesetId) -> Result<&[CommandCall], CoreError> {
        self.entries
            .get(id.as_str())
            .map(|entry| entry.inverse.as_slice())
            .ok_or_else(|| error::changeset_not_found(id.as_str()))
    }

    pub(crate) fn forward(&self, id: &ChangesetId) -> Result<&[CommandCall], CoreError> {
        self.entries
            .get(id.as_str())
            .map(|entry| entry.public.forward.as_slice())
            .ok_or_else(|| error::changeset_not_found(id.as_str()))
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
}
