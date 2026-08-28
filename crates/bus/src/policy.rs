//! Mutation policy keyed by trusted out-of-band [`Origin`].

use omacell_core::command::Origin;

/// Who may directly mutate, propose, apply, or invoke internal restores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MutationPolicy;

impl MutationPolicy {
    /// Model origins that may propose but cannot execute mutating commands.
    #[must_use]
    pub fn is_model_origin(origin: Origin) -> bool {
        matches!(
            origin,
            Origin::InAppAgent | Origin::ExternalAgent | Origin::PalettePlan
        )
    }

    /// Direct execution of a mutating command.
    #[must_use]
    pub fn allow_direct_mutate(origin: Origin) -> bool {
        !Self::is_model_origin(origin)
    }

    /// Submitting a changeset proposal.
    #[must_use]
    pub fn allow_propose(origin: Origin) -> bool {
        let _ = origin;
        true
    }

    /// Applying or reverting a stored changeset.
    #[must_use]
    pub fn allow_apply(origin: Origin) -> bool {
        !Self::is_model_origin(origin)
    }

    /// Using an internal restore as an external forward command.
    #[must_use]
    pub fn allow_internal_forward(_origin: Origin) -> bool {
        false
    }
}
