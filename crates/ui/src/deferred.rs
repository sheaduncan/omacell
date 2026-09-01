//! Command ids that appear in Appendix A maps but are owned by a later WP.

/// Ownership of a keymap command that this crate does not register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeferredCommand {
    /// Dotted command id.
    pub id: &'static str,
    /// Owning work package (`WP-17`).
    pub wp: &'static str,
}

/// Commands supplied by the executable composition root rather than a crate
/// that `omacell-ui` can depend on directly.
///
/// The CLI integration suite verifies that every id in this list is present in
/// the live catalog. Keeping this separate from [`DEFERRED_COMMANDS`] prevents
/// shipped commands from being mistaken for unfinished work.
pub const COMPOSITION_COMMANDS: &[&str] = &["file.open", "file.save", "file.print", "ai.plan"];

/// Tested deferred-command table. Empty only at the final integration gate.
pub const DEFERRED_COMMANDS: &[DeferredCommand] = &[
    DeferredCommand {
        id: "edit.searchnext",
        wp: "WP-19",
    },
    DeferredCommand {
        id: "edit.searchprev",
        wp: "WP-19",
    },
    DeferredCommand {
        id: "file.new",
        wp: "WP-16",
    },
    DeferredCommand {
        id: "file.close",
        wp: "WP-16",
    },
    DeferredCommand {
        id: "file.saveas",
        wp: "WP-16",
    },
    DeferredCommand {
        id: "name.manager",
        wp: "WP-18",
    },
    DeferredCommand {
        id: "name.paste",
        wp: "WP-18",
    },
    DeferredCommand {
        id: "name.createfrom",
        wp: "WP-18",
    },
    DeferredCommand {
        id: "edit.explainerror",
        wp: "WP-19",
    },
    DeferredCommand {
        id: "ai.assist",
        wp: "WP-23",
    },
    DeferredCommand {
        id: "chart.export",
        wp: "WP-25",
    },
];

/// Look up a deferred owner.
#[must_use]
pub fn owner(id: &str) -> Option<&'static str> {
    DEFERRED_COMMANDS.iter().find(|d| d.id == id).map(|d| d.wp)
}

/// Whether the executable composition root supplies `id`.
#[must_use]
pub fn is_composition_command(id: &str) -> bool {
    COMPOSITION_COMMANDS.contains(&id)
}
