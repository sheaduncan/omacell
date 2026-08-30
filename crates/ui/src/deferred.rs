//! Command ids that appear in Appendix A maps but are owned by a later WP.

/// Ownership of a keymap command that this crate does not register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeferredCommand {
    /// Dotted command id.
    pub id: &'static str,
    /// Owning work package (`WP-17`).
    pub wp: &'static str,
}

/// Tested deferred-command table. Empty only at the final integration gate.
pub const DEFERRED_COMMANDS: &[DeferredCommand] = &[
    DeferredCommand {
        id: "edit.cut",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.copy",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.paste",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.pastespecial",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.yank",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.delete",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.change",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.clearrow",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.repeat",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.fillselection",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.filldown",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.fillright",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.autosum",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.insertdate",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.inserttime",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.copyformulaabove",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.copyvalueabove",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.clearcell",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "format.indent",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "format.outdent",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.searchnext",
        wp: "WP-19",
    },
    DeferredCommand {
        id: "edit.searchprev",
        wp: "WP-19",
    },
    DeferredCommand {
        id: "file.open",
        wp: "WP-13",
    },
    DeferredCommand {
        id: "file.save",
        wp: "WP-13",
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
        id: "file.print",
        wp: "WP-26",
    },
    DeferredCommand {
        id: "format.panel",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "format.bold",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "format.italic",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "format.underline",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "format.general",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "format.numberstyle",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "format.time",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "format.date",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "format.currency",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "format.percent",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "format.scientific",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "format.borderoutline",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "format.bordernone",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "table.create",
        wp: "WP-18",
    },
    DeferredCommand {
        id: "edit.hyperlink",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "filter.toggle",
        wp: "WP-18",
    },
    DeferredCommand {
        id: "edit.insert",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.delcells",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "view.hiderows",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "view.hidecols",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "view.unhiderows",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "view.unhidecols",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.note",
        wp: "WP-17",
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
        id: "edit.group",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.ungroup",
        wp: "WP-17",
    },
    DeferredCommand {
        id: "edit.explainerror",
        wp: "WP-19",
    },
    DeferredCommand {
        id: "ai.plan",
        wp: "WP-23",
    },
    DeferredCommand {
        id: "ai.assist",
        wp: "WP-23",
    },
    DeferredCommand {
        id: "ai.agent",
        wp: "WP-21",
    },
    DeferredCommand {
        id: "chart.export",
        wp: "WP-25",
    },
    DeferredCommand {
        id: "edit.flashfill",
        wp: "WP-18",
    },
];

/// Look up a deferred owner.
#[must_use]
pub fn owner(id: &str) -> Option<&'static str> {
    DEFERRED_COMMANDS.iter().find(|d| d.id == id).map(|d| d.wp)
}
