//! WP-17 editing and structure commands.

use omacell_core::addr::{CellRef, RangeRef, col_to_letters};
use omacell_core::changeset::ChangeSummary;
use omacell_core::error::CoreError;
use omacell_core::ops::{
    ClipCell, ClipExtras, FillMode, PasteOp, PasteSpecial, Shift, TextColumnType,
    TextToColumnsMode, TextToColumnsPlan, autofit_columns, autofit_rows, autofit_width,
    consolidate_by_position, copy_extras, copy_range, delete_cells, delete_cols, delete_rows,
    detect_fill, excel_xor_hash, fill_custom_list, fill_range, insert_cells, insert_cols,
    insert_rows, merge, merge_across, move_range_cells_between, paste_extras, paste_special_from,
    remove_duplicates_with_header, text_to_columns_with_plan, unmerge,
};
use omacell_core::sheet::{Comment, Hyperlink, Note, ProtectedRange, ProtectionAllow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::handler::{CommandContext, Effect};
use crate::registry::{CommandKind, CommandRegistry, CommandSpec, Exposure};
use crate::resolve::{resolve_cell, resolve_range, resolve_range_unbounded};

/// `edit.insert` / `edit.delcells`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditInsertArgs {
    /// A1 range.
    pub range: String,
    /// `down` / `right` / `rows` / `cols`.
    #[serde(default = "down")]
    pub shift: String,
}

fn down() -> String {
    "down".into()
}

/// `edit.copy` / fill
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RangeOnlyArgs {
    /// A1 range.
    pub range: String,
}

/// `edit.paste`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditPasteArgs {
    /// Destination A1.
    pub range: String,
    /// Clipboard JSON from `edit.copy`.
    pub payload: serde_json::Value,
    /// Legacy mode string or a composable paste-special option object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub special: Option<PasteSpecialArg>,
}

/// Backward-compatible paste-special argument.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum PasteSpecialArg {
    /// Legacy single mode (`values`, `formulas`, `transpose`, ...).
    Name(String),
    /// Composable matrix options.
    Options(PasteSpecialOptions),
}

/// Composable paste-special matrix.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PasteSpecialOptions {
    /// Paste literal/cached values.
    #[serde(default)]
    pub values: bool,
    /// Paste formulas.
    #[serde(default)]
    pub formulas: bool,
    /// Paste complete styles.
    #[serde(default)]
    pub formats: bool,
    /// Paste number formats only.
    #[serde(default)]
    pub number_formats: bool,
    /// Paste source column widths.
    #[serde(default)]
    pub column_widths: bool,
    /// Transpose rows and columns.
    #[serde(default)]
    pub transpose: bool,
    /// Do not overwrite with blank sources.
    #[serde(default)]
    pub skip_blanks: bool,
    /// `add`, `subtract`, `multiply`, or `divide`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    /// Paste formulas linking to the source cells.
    #[serde(default)]
    pub paste_link: bool,
    /// Include notes, comments, links, and merges.
    #[serde(default)]
    pub include_objects: bool,
}

/// `edit.move`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditMoveArgs {
    /// Source A1 range.
    pub src: String,
    /// Destination A1 cell.
    pub dest: String,
    /// Copy instead of moving, for Ctrl+drag.
    #[serde(default)]
    pub copy: bool,
}

/// `edit.fillselection`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditFillArgs {
    /// Source A1 range.
    pub src: String,
    /// Destination A1 range.
    pub dest: String,
    /// `copy` / `linear` / `growth` / `date` / `weekday` / `month` / `year`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// User-defined list used as a cyclic series.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_list: Option<Vec<String>>,
}

/// `sheet.reorder`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SheetReorderArgs {
    /// Sheet name.
    pub sheet: String,
    /// 0-based target index.
    pub index: u32,
}

/// `sheet.protect`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SheetProtectArgs {
    /// Sheet name; default active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,
    /// Password (legacy XOR hash; not security).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Enable protection.
    #[serde(default = "on")]
    pub enable: bool,
    /// Actions users may still perform while protection is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow: Option<ProtectionAllowArgs>,
}

/// Sheet-protection allowed actions.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProtectionAllowArgs {
    /// Select locked cells.
    #[serde(default = "on")]
    pub select_locked: bool,
    /// Select unlocked cells.
    #[serde(default = "on")]
    pub select_unlocked: bool,
    /// Format cells.
    #[serde(default)]
    pub format_cells: bool,
    /// Insert rows.
    #[serde(default)]
    pub insert_rows: bool,
    /// Insert columns.
    #[serde(default)]
    pub insert_cols: bool,
    /// Sort ranges.
    #[serde(default)]
    pub sort: bool,
    /// Use AutoFilter.
    #[serde(default)]
    pub auto_filter: bool,
}

/// `workbook.protect` arguments.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkbookProtectArgs {
    /// Password (legacy XOR hash; not security).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Enable protection.
    #[serde(default = "on")]
    pub enable: bool,
    /// Protect workbook structure.
    #[serde(default = "on")]
    pub lock_structure: bool,
    /// Protect workbook windows.
    #[serde(default)]
    pub lock_windows: bool,
}

/// Cell locked/hidden flags.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CellProtectionArgs {
    /// A1 target range.
    pub range: String,
    /// Locked when sheet protection is active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    /// Hide formulas when sheet protection is active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
}

/// Add, replace, or remove a protected-range entry.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProtectedRangeArgs {
    /// Sheet name; active sheet when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,
    /// Entry name.
    pub name: String,
    /// One or more A1 ranges on the target sheet.
    #[serde(default)]
    pub ranges: Vec<String>,
    /// Optional legacy password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Remove the named entry.
    #[serde(default)]
    pub remove: bool,
}

fn on() -> bool {
    true
}

/// `edit.note`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditNoteArgs {
    /// A1 cell.
    #[serde(rename = "ref")]
    pub cell_ref: String,
    /// Note body. Empty deletes.
    pub text: String,
    /// Author.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

/// `edit.hyperlink`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditHyperlinkArgs {
    /// A1 cell.
    #[serde(rename = "ref")]
    pub cell_ref: String,
    /// URL or internal location. Empty deletes.
    pub target: String,
    /// Optional tooltip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    /// Optional display text override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
}

/// Create, replace, or delete a threaded comment.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditCommentArgs {
    /// A1 cell.
    #[serde(rename = "ref")]
    pub cell_ref: String,
    /// Author. Required when text is non-empty.
    #[serde(default)]
    pub author: String,
    /// Comment body; empty deletes the thread.
    pub text: String,
}

/// Add a reply to a threaded comment.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditCommentReplyArgs {
    /// A1 cell containing the thread.
    #[serde(rename = "ref")]
    pub cell_ref: String,
    /// Reply author.
    pub author: String,
    /// Reply body.
    pub text: String,
}

/// Resolve or reopen a threaded comment.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditCommentResolveArgs {
    /// A1 cell containing the thread.
    #[serde(rename = "ref")]
    pub cell_ref: String,
    /// Resolved state.
    #[serde(default = "on")]
    pub resolved: bool,
}

/// `range.removeduplicates`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveDupArgs {
    /// A1 range.
    pub range: String,
    /// Zero-based columns relative to the selected range; empty means all.
    #[serde(default)]
    pub columns: Vec<u16>,
    /// Preserve the first row as headers.
    #[serde(default)]
    pub has_headers: bool,
}

/// Consolidate-by-position arguments.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConsolidateArgs {
    /// Source A1 ranges, optionally sheet-qualified.
    pub sources: Vec<String>,
    /// Top-left destination cell.
    pub dest: String,
}

/// `edit.texttocolumns`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextToColumnsArgs {
    /// A1 range.
    pub range: String,
    /// Delimiter character.
    #[serde(default = "comma")]
    pub delim: String,
    /// `delimited` or `fixed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Delimiter characters for delimited mode; defaults to `delim`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delimiters: Option<String>,
    /// Unicode character offsets for fixed-width mode.
    #[serde(default)]
    pub breaks: Vec<usize>,
    /// Per-field `general`, `text`, or `skip` conversions.
    #[serde(default)]
    pub column_types: Vec<String>,
}

fn comma() -> String {
    ",".into()
}

/// Hide/group args.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AxisRangeArgs {
    /// A1 range whose rows or columns are targeted.
    pub range: String,
}

/// Explicit row height or column width.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AxisSizeArgs {
    /// A1 range whose rows or columns are targeted.
    pub range: String,
    /// Pixel size.
    pub px: u32,
}

/// Outline collapse/expand arguments.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AxisCollapseArgs {
    /// A1 range whose rows or columns are targeted.
    pub range: String,
    /// `rows` or `cols`; inferred from whole-column ranges when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis: Option<String>,
}

/// Clear selected record classes from a range.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditClearArgs {
    /// A1 range.
    pub range: String,
    /// `contents`, `formats`, `comments`, `hyperlinks`, or `all`.
    #[serde(default = "contents")]
    pub what: String,
}

/// Formatting action target and optional explicit toggle state.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FormatActionArgs {
    /// A1 target; current sheet selection when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
    /// Explicit on/off state; omitted toggles from the first cell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// Command target that defaults to the active selection.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SelectionArgs {
    /// A1 target; current sheet selection when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
}

/// Date/time insertion target.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InsertSerialArgs {
    /// A1 target; current sheet selection when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
    /// Explicit Excel serial, primarily for deterministic automation and tests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<f64>,
}

/// Repeat the last successful public mutation.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepeatArgs {
    /// Number of repetitions (bounded to 1,000).
    #[serde(default = "one")]
    pub count: u32,
}

fn one() -> u32 {
    1
}

fn contents() -> String {
    "contents".into()
}

/// Register WP-17 commands.
pub fn register_edit_commands(registry: &mut CommandRegistry) -> Result<(), CoreError> {
    crate::restore::register(registry)?;
    registry.register(
        CommandSpec {
            id: "edit.insert",
            doc: "Insert cells, rows, or columns",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+Shift+="],
        },
        edit_insert,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.delcells",
            doc: "Delete cells, rows, or columns",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+-"],
        },
        edit_delcells,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.copy",
            doc: "Copy a range to an internal clipboard payload",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+C"],
        },
        edit_copy,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.cut",
            doc: "Cut a range (copy payload marked for move)",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+X"],
        },
        edit_cut,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.yank",
            doc: "Copy the active selection to an internal clipboard payload",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        edit_yank,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.paste",
            doc: "Paste a clipboard payload",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+V"],
        },
        edit_paste,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.pastespecial",
            doc: "Paste special",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+Alt+V"],
        },
        edit_paste,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.move",
            doc: "Move a range in one atomic operation",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        edit_move,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.fillselection",
            doc: "Fill the destination from a source range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+Enter"],
        },
        edit_fill,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.filldown",
            doc: "Fill down",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+D"],
        },
        edit_filldown,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.fillright",
            doc: "Fill right",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+R"],
        },
        edit_fillright,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.fillup",
            doc: "Fill up",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        edit_fillup,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.fillleft",
            doc: "Fill left",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        edit_fillleft,
    )?;
    registry.register(
        CommandSpec {
            id: "range.merge",
            doc: "Merge a range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        range_merge,
    )?;
    registry.register(
        CommandSpec {
            id: "range.mergeacross",
            doc: "Merge each row of a range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        range_mergeacross,
    )?;
    registry.register(
        CommandSpec {
            id: "range.unmerge",
            doc: "Unmerge overlapping merges",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        range_unmerge,
    )?;
    registry.register(
        CommandSpec {
            id: "view.hiderows",
            doc: "Hide rows in a range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+9"],
        },
        hide_rows,
    )?;
    registry.register(
        CommandSpec {
            id: "view.hidecols",
            doc: "Hide columns in a range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+0"],
        },
        hide_cols,
    )?;
    registry.register(
        CommandSpec {
            id: "view.unhiderows",
            doc: "Unhide rows",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+Shift+9"],
        },
        unhide_rows,
    )?;
    registry.register(
        CommandSpec {
            id: "view.unhidecols",
            doc: "Unhide columns",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+Shift+0"],
        },
        unhide_cols,
    )?;
    registry.register(
        CommandSpec {
            id: "format.rowheight",
            doc: "Set row height in pixels",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        set_row_height,
    )?;
    registry.register(
        CommandSpec {
            id: "format.colwidth",
            doc: "Set column width in pixels",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        set_col_width,
    )?;
    registry.register(
        CommandSpec {
            id: "format.autofitrows",
            doc: "Auto-fit row heights",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        auto_fit_rows,
    )?;
    registry.register(
        CommandSpec {
            id: "format.autofitcols",
            doc: "Auto-fit column widths",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        auto_fit_cols,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.group",
            doc: "Group rows or columns",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Alt+Shift+Right"],
        },
        edit_group,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.ungroup",
            doc: "Ungroup rows or columns",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Alt+Shift+Left"],
        },
        edit_ungroup,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.collapse",
            doc: "Collapse a row or column outline group",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        edit_collapse,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.expand",
            doc: "Expand a row or column outline group",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        edit_expand,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.note",
            doc: "Set or clear a legacy note",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Shift+F2"],
        },
        edit_note,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.comment",
            doc: "Create, replace, or delete a threaded comment",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        edit_comment,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.commentreply",
            doc: "Reply to a threaded comment",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        edit_comment_reply,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.commentresolve",
            doc: "Resolve or reopen a threaded comment",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        edit_comment_resolve,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.hyperlink",
            doc: "Set or clear a hyperlink",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+K"],
        },
        edit_hyperlink,
    )?;
    registry.register(
        CommandSpec {
            id: "sheet.protect",
            doc: "Protect or unprotect a sheet (legacy XOR hash)",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        sheet_protect,
    )?;
    registry.register(
        CommandSpec {
            id: "workbook.protect",
            doc: "Protect or unprotect workbook structure and windows",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        workbook_protect,
    )?;
    registry.register(
        CommandSpec {
            id: "sheet.protectedrange",
            doc: "Add, replace, or remove a protected-range entry",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        sheet_protected_range,
    )?;
    registry.register(
        CommandSpec {
            id: "format.protection",
            doc: "Set locked and hidden flags on cells",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        format_protection,
    )?;
    registry.register(
        CommandSpec {
            id: "sheet.reorder",
            doc: "Move a sheet to a new tab index",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        sheet_reorder,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.texttocolumns",
            doc: "Split text into columns",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        edit_texttocolumns,
    )?;
    registry.register(
        CommandSpec {
            id: "range.removeduplicates",
            doc: "Clear duplicate rows in a range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        range_removeduplicates,
    )?;
    registry.register(
        CommandSpec {
            id: "range.consolidate",
            doc: "Consolidate source ranges by position using SUM",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        range_consolidate,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.clearcell",
            doc: "Clear cell contents in a range",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &["Delete"],
        },
        edit_clearcell,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.clear",
            doc: "Clear contents, formats, comments, hyperlinks, or all records",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        edit_clear,
    )?;
    for &(id, doc, handler) in &[
        (
            "edit.delete",
            "Delete the active selection and return its clipboard payload",
            edit_delete
                as for<'a> fn(&mut CommandContext<'a>, SelectionArgs) -> Result<Effect, CoreError>,
        ),
        (
            "edit.change",
            "Delete the active selection and request edit mode",
            edit_change,
        ),
        (
            "edit.clearrow",
            "Clear all records in the selected rows",
            edit_clear_row,
        ),
        (
            "edit.autosum",
            "Insert SUM formulas for the active selection",
            edit_autosum,
        ),
        (
            "edit.copyformulaabove",
            "Copy formulas from the row above",
            edit_copy_formula_above,
        ),
        (
            "edit.copyvalueabove",
            "Copy values from the row above",
            edit_copy_value_above,
        ),
    ] {
        registry.register(
            CommandSpec {
                id,
                doc,
                kind: CommandKind::Mutating,
                changeset_eligible: true,
                exposure: Exposure::Public,
                default_keys: &[],
            },
            handler,
        )?;
    }
    registry.register(
        CommandSpec {
            id: "edit.insertdate",
            doc: "Insert the current date or an explicit Excel serial",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        edit_insert_date,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.inserttime",
            doc: "Insert the current time or an explicit Excel serial",
            kind: CommandKind::Mutating,
            changeset_eligible: true,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        edit_insert_time,
    )?;
    registry.register(
        CommandSpec {
            id: "edit.repeat",
            doc: "Repeat the last successful public mutation",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        edit_repeat_placeholder,
    )?;
    type FormatHandler =
        for<'a> fn(&mut CommandContext<'a>, FormatActionArgs) -> Result<Effect, CoreError>;
    let format_commands: &[(&str, &str, FormatHandler)] = &[
        ("format.bold", "Toggle bold", format_bold),
        ("format.italic", "Toggle italic", format_italic),
        ("format.underline", "Toggle underline", format_underline),
        ("format.indent", "Increase indentation", format_indent),
        ("format.outdent", "Decrease indentation", format_outdent),
        (
            "format.general",
            "Apply General number format",
            format_general,
        ),
        (
            "format.numberstyle",
            "Apply Number format",
            format_numberstyle,
        ),
        ("format.time", "Apply Time format", format_time),
        ("format.date", "Apply Date format", format_date),
        ("format.currency", "Apply Currency format", format_currency),
        ("format.percent", "Apply Percent format", format_percent),
        (
            "format.scientific",
            "Apply Scientific format",
            format_scientific,
        ),
        (
            "format.borderoutline",
            "Apply an outline border",
            format_border_outline,
        ),
        ("format.bordernone", "Remove borders", format_border_none),
    ];
    for &(id, doc, handler) in format_commands {
        registry.register(
            CommandSpec {
                id,
                doc,
                kind: CommandKind::Mutating,
                changeset_eligible: true,
                exposure: Exposure::Public,
                default_keys: &[],
            },
            handler,
        )?;
    }
    registry.register(
        CommandSpec {
            id: "format.panel",
            doc: "Describe the current selection format",
            kind: CommandKind::Query,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+1"],
        },
        format_panel,
    )?;
    Ok(())
}

fn to_range(r: crate::resolve::ResolvedRange) -> Result<RangeRef, CoreError> {
    Ok(RangeRef::from_corners(
        CellRef::new(r.min_row, r.min_col)?,
        CellRef::new(r.max_row, r.max_col)?,
    ))
}

fn parse_shift(value: &str) -> Result<Shift, CoreError> {
    if value.eq_ignore_ascii_case("right") || value.eq_ignore_ascii_case("cols") {
        Ok(Shift::Right)
    } else if value.eq_ignore_ascii_case("down") || value.eq_ignore_ascii_case("rows") {
        Ok(Shift::Down)
    } else {
        Err(crate::error::args(format!(
            "unknown insert/delete shift {value:?}"
        )))
    }
}

fn whole_rows(r: crate::resolve::ResolvedRange) -> bool {
    r.min_col == 0 && r.max_col == omacell_core::limits::MAX_COLS - 1
}

fn whole_cols(r: crate::resolve::ResolvedRange) -> bool {
    r.min_row == 0 && r.max_row == omacell_core::limits::MAX_ROWS - 1
}

fn edit_insert(ctx: &mut CommandContext<'_>, args: EditInsertArgs) -> Result<Effect, CoreError> {
    let r = resolve_range_unbounded(ctx.workbook_ref(), &args.range)?;
    let shift = parse_shift(&args.shift)?;
    if whole_rows(r) || args.shift.eq_ignore_ascii_case("rows") {
        insert_rows(
            ctx.workbook(),
            r.sheet,
            r.min_row,
            r.max_row - r.min_row + 1,
        )?;
    } else if whole_cols(r) || args.shift.eq_ignore_ascii_case("cols") {
        insert_cols(
            ctx.workbook(),
            r.sheet,
            r.min_col,
            r.max_col - r.min_col + 1,
        )?;
    } else {
        if r.area() > crate::resolve::MAX_RANGE_CELLS {
            return Err(crate::error::range_size(r.area()));
        }
        insert_cells(ctx.workbook(), r.sheet, to_range(r)?, shift)?;
    }
    Ok(changed("insert"))
}

fn edit_delcells(ctx: &mut CommandContext<'_>, args: EditInsertArgs) -> Result<Effect, CoreError> {
    let r = resolve_range_unbounded(ctx.workbook_ref(), &args.range)?;
    let shift = parse_shift(&args.shift)?;
    if whole_rows(r) || args.shift.eq_ignore_ascii_case("rows") {
        delete_rows(
            ctx.workbook(),
            r.sheet,
            r.min_row,
            r.max_row - r.min_row + 1,
        )?;
    } else if whole_cols(r) || args.shift.eq_ignore_ascii_case("cols") {
        delete_cols(
            ctx.workbook(),
            r.sheet,
            r.min_col,
            r.max_col - r.min_col + 1,
        )?;
    } else {
        if r.area() > crate::resolve::MAX_RANGE_CELLS {
            return Err(crate::error::range_size(r.area()));
        }
        delete_cells(ctx.workbook(), r.sheet, to_range(r)?, shift)?;
    }
    Ok(changed("delete"))
}

fn edit_copy(ctx: &mut CommandContext<'_>, args: RangeOnlyArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    copy_effect(ctx, r, false)
}

fn edit_yank(ctx: &mut CommandContext<'_>, args: SelectionArgs) -> Result<Effect, CoreError> {
    let range = action_range(ctx, args.range.as_deref())?;
    copy_effect(ctx, range, false)
}

fn copy_effect(
    ctx: &CommandContext<'_>,
    r: crate::resolve::ResolvedRange,
    cut: bool,
) -> Result<Effect, CoreError> {
    let range = to_range(r)?;
    let grid = copy_range(ctx.workbook_ref(), r.sheet, range);
    let extras = copy_extras(ctx.workbook_ref(), r.sheet, range);
    Ok(Effect::query(serde_json::json!({
        "payload": {
            "cut": cut,
            "sheet": r.sheet.index(),
            "row": r.min_row,
            "col": r.min_col,
            "cells": grid,
            "extras": extras,
        }
    })))
}

fn edit_cut(ctx: &mut CommandContext<'_>, args: RangeOnlyArgs) -> Result<Effect, CoreError> {
    let range = resolve_range(ctx.workbook_ref(), &args.range)?;
    copy_effect(ctx, range, true)
}

fn edit_paste(ctx: &mut CommandContext<'_>, args: EditPasteArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    let cells = args
        .payload
        .get("cells")
        .cloned()
        .ok_or_else(|| crate::error::args("clipboard payload is missing cells"))?;
    let grid: Vec<Vec<ClipCell>> = serde_json::from_value(cells)
        .map_err(|error| crate::error::args(format!("invalid clipboard cells: {error}")))?;
    let extras: ClipExtras = match args.payload.get("extras").cloned() {
        Some(value) => serde_json::from_value(value)
            .map_err(|error| crate::error::args(format!("invalid clipboard metadata: {error}")))?,
        None => ClipExtras::default(),
    };
    let origin = match (
        args.payload.get("row").and_then(|value| value.as_u64()),
        args.payload.get("col").and_then(|value| value.as_u64()),
    ) {
        (Some(row), Some(col)) => Some((
            u32::try_from(row).map_err(|_| crate::error::args("clipboard row is out of range"))?,
            u16::try_from(col)
                .map_err(|_| crate::error::args("clipboard column is out of range"))?,
        )),
        _ => None,
    };
    let payload_sheet = args
        .payload
        .get("sheet")
        .and_then(|value| value.as_u64())
        .map(|value| {
            u32::try_from(value)
                .map(omacell_core::addr::SheetId::new)
                .map_err(|_| crate::error::args("clipboard sheet is out of range"))
        })
        .transpose()?;
    if args
        .payload
        .get("cut")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let (Some((source_row, source_col)), Some(source_sheet)) = (origin, payload_sheet) else {
            return Err(crate::error::args(
                "cut payload is missing its source location",
            ));
        };
        let height =
            u32::try_from(grid.len()).map_err(|_| crate::error::args("clipboard is too tall"))?;
        let width = grid.iter().map(Vec::len).max().unwrap_or(0);
        let width =
            u16::try_from(width).map_err(|_| crate::error::args("clipboard is too wide"))?;
        if height == 0 || width == 0 {
            return Ok(Effect::query(serde_json::json!({"changed": 0})));
        }
        let source_end = CellRef::new(
            source_row
                .checked_add(height - 1)
                .ok_or_else(|| crate::error::args("cut source row overflow"))?,
            source_col
                .checked_add(width - 1)
                .ok_or_else(|| crate::error::args("cut source column overflow"))?,
        )?;
        let src = RangeRef::from_corners(CellRef::new(source_row, source_col)?, source_end);
        let changed = move_range_cells_between(
            ctx.workbook(),
            source_sheet,
            src,
            r.sheet,
            CellRef::new(r.min_row, r.min_col)?,
        )?;
        return Ok(Effect {
            result: serde_json::json!({"changed": changed}),
            auto_recalc: true,
            rebuild: true,
            summary: ChangeSummary {
                cells: u64::from(changed),
                text: "cut/paste".into(),
                ..ChangeSummary::default()
            },
            ..Effect::default()
        });
    }
    let (spec, include_objects) = paste_options(args.special.as_ref())?;
    let n = paste_special_from(
        ctx.workbook(),
        r.sheet,
        CellRef::new(r.min_row, r.min_col)?,
        &grid,
        spec,
        origin,
        payload_sheet,
    )?;
    if include_objects {
        paste_extras(
            ctx.workbook(),
            r.sheet,
            CellRef::new(r.min_row, r.min_col)?,
            &extras,
            spec.transpose,
        )?;
    }
    Ok(Effect {
        result: serde_json::json!({"changed": n}),
        auto_recalc: true,
        summary: ChangeSummary {
            cells: u64::from(n),
            text: "paste".into(),
            ..ChangeSummary::default()
        },
        ..Effect::default()
    })
}

fn paste_options(special: Option<&PasteSpecialArg>) -> Result<(PasteSpecial, bool), CoreError> {
    let ordinary = PasteSpecial {
        values: true,
        formulas: true,
        formats: true,
        ..PasteSpecial::default()
    };
    let Some(special) = special else {
        return Ok((ordinary, true));
    };
    match special {
        PasteSpecialArg::Name(kind) => {
            let spec = match kind.as_str() {
                "values" => PasteSpecial {
                    values: true,
                    ..PasteSpecial::default()
                },
                "formulas" => PasteSpecial {
                    formulas: true,
                    ..PasteSpecial::default()
                },
                "transpose" => PasteSpecial {
                    values: true,
                    transpose: true,
                    ..PasteSpecial::default()
                },
                "add" => PasteSpecial {
                    operation: PasteOp::Add,
                    ..PasteSpecial::default()
                },
                "skipblanks" => PasteSpecial {
                    values: true,
                    skip_blanks: true,
                    ..PasteSpecial::default()
                },
                "link" => PasteSpecial {
                    paste_link: true,
                    ..PasteSpecial::default()
                },
                "formats" => PasteSpecial {
                    formats: true,
                    ..PasteSpecial::default()
                },
                "numberformats" => PasteSpecial {
                    number_formats: true,
                    ..PasteSpecial::default()
                },
                "columnwidths" => PasteSpecial {
                    column_widths: true,
                    ..PasteSpecial::default()
                },
                "subtract" => PasteSpecial {
                    operation: PasteOp::Sub,
                    ..PasteSpecial::default()
                },
                "multiply" => PasteSpecial {
                    operation: PasteOp::Mul,
                    ..PasteSpecial::default()
                },
                "divide" => PasteSpecial {
                    operation: PasteOp::Div,
                    ..PasteSpecial::default()
                },
                other => {
                    return Err(crate::error::args(format!(
                        "unknown paste-special mode {other:?}"
                    )));
                }
            };
            Ok((spec, kind == "transpose"))
        }
        PasteSpecialArg::Options(options) => {
            let operation = match options.operation.as_deref() {
                None => PasteOp::None,
                Some("add") => PasteOp::Add,
                Some("subtract") => PasteOp::Sub,
                Some("multiply") => PasteOp::Mul,
                Some("divide") => PasteOp::Div,
                Some(other) => {
                    return Err(crate::error::args(format!(
                        "unknown paste operation {other:?}"
                    )));
                }
            };
            Ok((
                PasteSpecial {
                    values: options.values,
                    formulas: options.formulas,
                    formats: options.formats,
                    number_formats: options.number_formats,
                    column_widths: options.column_widths,
                    transpose: options.transpose,
                    skip_blanks: options.skip_blanks,
                    operation,
                    paste_link: options.paste_link,
                },
                options.include_objects,
            ))
        }
    }
}

fn edit_move(ctx: &mut CommandContext<'_>, args: EditMoveArgs) -> Result<Effect, CoreError> {
    let src = resolve_range(ctx.workbook_ref(), &args.src)?;
    let dest = resolve_cell(ctx.workbook_ref(), &args.dest)?;
    let destination = CellRef::new(dest.row, dest.col)?;
    let n = if args.copy {
        let range = to_range(src)?;
        let grid = copy_range(ctx.workbook_ref(), src.sheet, range);
        let extras = copy_extras(ctx.workbook_ref(), src.sheet, range);
        let count = paste_special_from(
            ctx.workbook(),
            dest.sheet,
            destination,
            &grid,
            PasteSpecial {
                values: true,
                formulas: true,
                formats: true,
                ..PasteSpecial::default()
            },
            Some((src.min_row, src.min_col)),
            Some(src.sheet),
        )?;
        paste_extras(ctx.workbook(), dest.sheet, destination, &extras, false)?;
        count
    } else {
        move_range_cells_between(
            ctx.workbook(),
            src.sheet,
            to_range(src)?,
            dest.sheet,
            destination,
        )?
    };
    Ok(Effect {
        result: serde_json::json!({"changed": n}),
        auto_recalc: true,
        rebuild: true,
        summary: ChangeSummary {
            cells: u64::from(n),
            text: if args.copy { "copy" } else { "move" }.into(),
            ..ChangeSummary::default()
        },
        ..Effect::default()
    })
}

fn parse_mode(mode: Option<&str>, values: &[f64]) -> Result<FillMode, CoreError> {
    Ok(match mode {
        None => detect_fill(values),
        Some("linear") => FillMode::Linear,
        Some("growth") => FillMode::Growth,
        Some("date") => FillMode::Date,
        Some("weekday") => FillMode::Weekday,
        Some("month") => FillMode::Month,
        Some("year") => FillMode::Year,
        Some("copy") => FillMode::Copy,
        Some("formats") => FillMode::Formats,
        Some(other) => {
            return Err(crate::error::args(format!("unknown fill mode {other:?}")));
        }
    })
}

fn edit_fill(ctx: &mut CommandContext<'_>, args: EditFillArgs) -> Result<Effect, CoreError> {
    let src = resolve_range(ctx.workbook_ref(), &args.src)?;
    let dest = resolve_range(ctx.workbook_ref(), &args.dest)?;
    if src.sheet != dest.sheet {
        return Err(crate::error::args(
            "cross-sheet fill ranges are not supported by WP-17",
        ));
    }
    let mut nums = Vec::new();
    for r in src.min_row..=src.max_row {
        if let Ok(Some(slot)) = ctx.workbook_ref().get(src.sheet, r, src.min_col)
            && let omacell_core::value::Value::Number(n) = slot.value
        {
            nums.push(n);
        }
    }
    let n = if let Some(list) = args.custom_list.as_deref() {
        fill_custom_list(
            ctx.workbook(),
            src.sheet,
            to_range(src)?,
            to_range(dest)?,
            list,
        )?
    } else {
        let mode = parse_mode(args.mode.as_deref(), &nums)?;
        fill_range(
            ctx.workbook(),
            src.sheet,
            to_range(src)?,
            to_range(dest)?,
            mode,
        )?
    };
    Ok(Effect {
        result: serde_json::json!({"changed": n}),
        auto_recalc: true,
        ..Effect::default()
    })
}

fn edit_filldown(ctx: &mut CommandContext<'_>, args: RangeOnlyArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    if r.max_row == r.min_row {
        return Ok(changed("fill"));
    }
    let src = RangeRef::from_corners(
        CellRef::new(r.min_row, r.min_col)?,
        CellRef::new(r.min_row, r.max_col)?,
    );
    let n = fill_range(ctx.workbook(), r.sheet, src, to_range(r)?, FillMode::Copy)?;
    Ok(Effect {
        result: serde_json::json!({"changed": n}),
        auto_recalc: true,
        ..Effect::default()
    })
}

fn edit_fillright(ctx: &mut CommandContext<'_>, args: RangeOnlyArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    let src = RangeRef::from_corners(
        CellRef::new(r.min_row, r.min_col)?,
        CellRef::new(r.max_row, r.min_col)?,
    );
    let n = fill_range(ctx.workbook(), r.sheet, src, to_range(r)?, FillMode::Copy)?;
    Ok(Effect {
        result: serde_json::json!({"changed": n}),
        auto_recalc: true,
        ..Effect::default()
    })
}

fn edit_fillup(ctx: &mut CommandContext<'_>, args: RangeOnlyArgs) -> Result<Effect, CoreError> {
    let range = resolve_range(ctx.workbook_ref(), &args.range)?;
    if range.max_row == range.min_row {
        return Ok(changed("fill"));
    }
    let src = RangeRef::from_corners(
        CellRef::new(range.max_row, range.min_col)?,
        CellRef::new(range.max_row, range.max_col)?,
    );
    let count = fill_range(
        ctx.workbook(),
        range.sheet,
        src,
        to_range(range)?,
        FillMode::Copy,
    )?;
    Ok(Effect {
        result: serde_json::json!({"changed": count}),
        ..changed("fill up")
    })
}

fn edit_fillleft(ctx: &mut CommandContext<'_>, args: RangeOnlyArgs) -> Result<Effect, CoreError> {
    let range = resolve_range(ctx.workbook_ref(), &args.range)?;
    if range.max_col == range.min_col {
        return Ok(changed("fill"));
    }
    let src = RangeRef::from_corners(
        CellRef::new(range.min_row, range.max_col)?,
        CellRef::new(range.max_row, range.max_col)?,
    );
    let count = fill_range(
        ctx.workbook(),
        range.sheet,
        src,
        to_range(range)?,
        FillMode::Copy,
    )?;
    Ok(Effect {
        result: serde_json::json!({"changed": count}),
        ..changed("fill left")
    })
}

fn range_merge(ctx: &mut CommandContext<'_>, args: RangeOnlyArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    merge(ctx.workbook(), r.sheet, to_range(r)?)?;
    Ok(changed("merge"))
}

fn range_mergeacross(
    ctx: &mut CommandContext<'_>,
    args: RangeOnlyArgs,
) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    merge_across(ctx.workbook(), r.sheet, to_range(r)?)?;
    Ok(changed("merge"))
}

fn range_unmerge(ctx: &mut CommandContext<'_>, args: RangeOnlyArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    let n = unmerge(ctx.workbook(), r.sheet, to_range(r)?)?;
    Ok(Effect {
        result: serde_json::json!({"changed": n}),
        ..Effect::default()
    })
}

fn hide_rows(ctx: &mut CommandContext<'_>, args: AxisRangeArgs) -> Result<Effect, CoreError> {
    let r = resolve_range_unbounded(ctx.workbook_ref(), &args.range)?;
    for row in r.min_row..=r.max_row {
        ctx.workbook().set_row_hidden(r.sheet, row, true)?;
    }
    Ok(changed("hide"))
}

fn hide_cols(ctx: &mut CommandContext<'_>, args: AxisRangeArgs) -> Result<Effect, CoreError> {
    let r = resolve_range_unbounded(ctx.workbook_ref(), &args.range)?;
    for col in r.min_col..=r.max_col {
        ctx.workbook().set_col_hidden(r.sheet, col, true)?;
    }
    Ok(changed("hide"))
}

fn unhide_rows(ctx: &mut CommandContext<'_>, args: AxisRangeArgs) -> Result<Effect, CoreError> {
    let r = resolve_range_unbounded(ctx.workbook_ref(), &args.range)?;
    for row in r.min_row..=r.max_row {
        ctx.workbook().set_row_hidden(r.sheet, row, false)?;
    }
    Ok(changed("unhide"))
}

fn unhide_cols(ctx: &mut CommandContext<'_>, args: AxisRangeArgs) -> Result<Effect, CoreError> {
    let r = resolve_range_unbounded(ctx.workbook_ref(), &args.range)?;
    for col in r.min_col..=r.max_col {
        ctx.workbook().set_col_hidden(r.sheet, col, false)?;
    }
    Ok(changed("unhide"))
}

fn set_row_height(ctx: &mut CommandContext<'_>, args: AxisSizeArgs) -> Result<Effect, CoreError> {
    let range = resolve_range_unbounded(ctx.workbook_ref(), &args.range)?;
    for row in range.min_row..=range.max_row {
        ctx.workbook().set_row_height(range.sheet, row, args.px)?;
    }
    Ok(changed("row height"))
}

fn set_col_width(ctx: &mut CommandContext<'_>, args: AxisSizeArgs) -> Result<Effect, CoreError> {
    let range = resolve_range_unbounded(ctx.workbook_ref(), &args.range)?;
    for col in range.min_col..=range.max_col {
        ctx.workbook().set_col_width(range.sheet, col, args.px)?;
    }
    Ok(changed("column width"))
}

fn auto_fit_rows(ctx: &mut CommandContext<'_>, args: AxisRangeArgs) -> Result<Effect, CoreError> {
    let range = resolve_range_unbounded(ctx.workbook_ref(), &args.range)?;
    let count = autofit_rows(
        ctx.workbook(),
        range.sheet,
        to_range(range)?,
        |text, style| {
            let lines = text.lines().count().max(1) as u32;
            let scale = (style.font.size_pt / 11.0).max(0.5);
            (f64::from(lines * 20) * scale).ceil() as u32
        },
    )?;
    Ok(Effect {
        result: serde_json::json!({"changed": count}),
        ..changed("auto-fit rows")
    })
}

fn auto_fit_cols(ctx: &mut CommandContext<'_>, args: AxisRangeArgs) -> Result<Effect, CoreError> {
    let range = resolve_range_unbounded(ctx.workbook_ref(), &args.range)?;
    let count = autofit_columns(
        ctx.workbook(),
        range.sheet,
        to_range(range)?,
        |text, style| {
            let scale = (style.font.size_pt / 11.0).max(0.5);
            (f64::from(autofit_width(text)) * scale).ceil() as u32
        },
    )?;
    Ok(Effect {
        result: serde_json::json!({"changed": count}),
        ..changed("auto-fit columns")
    })
}

fn edit_group(ctx: &mut CommandContext<'_>, args: AxisRangeArgs) -> Result<Effect, CoreError> {
    let r = resolve_range_unbounded(ctx.workbook_ref(), &args.range)?;
    if whole_cols(r) {
        for c in r.min_col..=r.max_col {
            let level = ctx
                .workbook_ref()
                .col_outline_level(r.sheet, c)?
                .saturating_add(1);
            ctx.workbook().set_col_outline_level(r.sheet, c, level)?;
        }
    } else {
        for row in r.min_row..=r.max_row {
            let level = ctx
                .workbook_ref()
                .row_outline_level(r.sheet, row)?
                .saturating_add(1);
            ctx.workbook().set_row_outline_level(r.sheet, row, level)?;
        }
    }
    Ok(changed("group"))
}

fn edit_ungroup(ctx: &mut CommandContext<'_>, args: AxisRangeArgs) -> Result<Effect, CoreError> {
    let r = resolve_range_unbounded(ctx.workbook_ref(), &args.range)?;
    if whole_cols(r) {
        for col in r.min_col..=r.max_col {
            let level = ctx
                .workbook_ref()
                .col_outline_level(r.sheet, col)?
                .saturating_sub(1);
            ctx.workbook().set_col_outline_level(r.sheet, col, level)?;
        }
    } else {
        for row in r.min_row..=r.max_row {
            let level = ctx
                .workbook_ref()
                .row_outline_level(r.sheet, row)?
                .saturating_sub(1);
            ctx.workbook().set_row_outline_level(r.sheet, row, level)?;
        }
    }
    Ok(changed("ungroup"))
}

fn edit_collapse(
    ctx: &mut CommandContext<'_>,
    args: AxisCollapseArgs,
) -> Result<Effect, CoreError> {
    set_collapsed(ctx, args, true)
}

fn edit_expand(ctx: &mut CommandContext<'_>, args: AxisCollapseArgs) -> Result<Effect, CoreError> {
    set_collapsed(ctx, args, false)
}

fn set_collapsed(
    ctx: &mut CommandContext<'_>,
    args: AxisCollapseArgs,
    collapsed: bool,
) -> Result<Effect, CoreError> {
    let range = resolve_range_unbounded(ctx.workbook_ref(), &args.range)?;
    let columns = match args.axis.as_deref() {
        None => whole_cols(range),
        Some("cols") => true,
        Some("rows") => false,
        Some(other) => {
            return Err(crate::error::args(format!(
                "outline axis must be rows or cols (got {other:?})"
            )));
        }
    };
    if columns {
        for col in range.min_col..=range.max_col {
            ctx.workbook()
                .set_col_collapsed(range.sheet, col, collapsed)?;
        }
    } else {
        for row in range.min_row..=range.max_row {
            ctx.workbook()
                .set_row_collapsed(range.sheet, row, collapsed)?;
        }
    }
    Ok(changed(if collapsed { "collapse" } else { "expand" }))
}

fn edit_note(ctx: &mut CommandContext<'_>, args: EditNoteArgs) -> Result<Effect, CoreError> {
    let c = resolve_cell(ctx.workbook_ref(), &args.cell_ref)?;
    let note = if args.text.is_empty() {
        None
    } else {
        Some(Note {
            author: args.author,
            text: args.text,
        })
    };
    ctx.workbook().set_note(c.sheet, c.row, c.col, note)?;
    Ok(changed("note"))
}

fn edit_comment(ctx: &mut CommandContext<'_>, args: EditCommentArgs) -> Result<Effect, CoreError> {
    let cell = resolve_cell(ctx.workbook_ref(), &args.cell_ref)?;
    let comment = if args.text.is_empty() {
        None
    } else {
        if args.author.trim().is_empty() {
            return Err(crate::error::args(
                "threaded comment author must not be empty",
            ));
        }
        Some(Comment {
            author: args.author,
            text: args.text,
            replies: Vec::new(),
            resolved: false,
        })
    };
    ctx.workbook()
        .set_comment(cell.sheet, cell.row, cell.col, comment)?;
    Ok(changed("threaded comment"))
}

fn edit_comment_reply(
    ctx: &mut CommandContext<'_>,
    args: EditCommentReplyArgs,
) -> Result<Effect, CoreError> {
    if args.author.trim().is_empty() || args.text.is_empty() {
        return Err(crate::error::args(
            "threaded comment replies require a non-empty author and body",
        ));
    }
    let cell = resolve_cell(ctx.workbook_ref(), &args.cell_ref)?;
    let mut thread = ctx
        .workbook_ref()
        .sheet(cell.sheet)
        .and_then(|sheet| sheet.comments.get(&(cell.row, cell.col)))
        .cloned()
        .ok_or_else(|| crate::error::args("threaded comment does not exist"))?;
    thread.replies.push(Comment {
        author: args.author,
        text: args.text,
        replies: Vec::new(),
        resolved: false,
    });
    ctx.workbook()
        .set_comment(cell.sheet, cell.row, cell.col, Some(thread))?;
    Ok(changed("comment reply"))
}

fn edit_comment_resolve(
    ctx: &mut CommandContext<'_>,
    args: EditCommentResolveArgs,
) -> Result<Effect, CoreError> {
    let cell = resolve_cell(ctx.workbook_ref(), &args.cell_ref)?;
    let mut thread = ctx
        .workbook_ref()
        .sheet(cell.sheet)
        .and_then(|sheet| sheet.comments.get(&(cell.row, cell.col)))
        .cloned()
        .ok_or_else(|| crate::error::args("threaded comment does not exist"))?;
    thread.resolved = args.resolved;
    ctx.workbook()
        .set_comment(cell.sheet, cell.row, cell.col, Some(thread))?;
    Ok(changed(if args.resolved {
        "resolve comment"
    } else {
        "reopen comment"
    }))
}

fn edit_hyperlink(
    ctx: &mut CommandContext<'_>,
    args: EditHyperlinkArgs,
) -> Result<Effect, CoreError> {
    let c = resolve_cell(ctx.workbook_ref(), &args.cell_ref)?;
    let hyperlink = if args.target.is_empty() {
        None
    } else {
        Some(Hyperlink {
            target: args.target,
            tooltip: args.tooltip,
            display: args.display,
        })
    };
    ctx.workbook()
        .set_hyperlink(c.sheet, c.row, c.col, hyperlink)?;
    Ok(changed("hyperlink"))
}

fn sheet_protect(
    ctx: &mut CommandContext<'_>,
    args: SheetProtectArgs,
) -> Result<Effect, CoreError> {
    let id = match args.sheet.as_deref() {
        Some(name) => ctx
            .workbook_ref()
            .sheet_by_name(name)
            .map(|s| s.id)
            .ok_or_else(|| CoreError::sheet_id("unknown sheet"))?,
        None => ctx.workbook_ref().active_sheet(),
    };
    let hash = args.password.as_deref().map(excel_xor_hash);
    let mut protection = ctx
        .workbook_ref()
        .sheet(id)
        .map(|sheet| sheet.protection.clone())
        .ok_or_else(|| CoreError::sheet_id("unknown sheet"))?;
    protection.enabled = args.enable;
    protection.password = hash.map(|value| format!("{value:04X}").into_bytes());
    if let Some(allow) = args.allow {
        protection.allow = ProtectionAllow {
            select_locked: allow.select_locked,
            select_unlocked: allow.select_unlocked,
            format_cells: allow.format_cells,
            insert_rows: allow.insert_rows,
            insert_cols: allow.insert_cols,
            sort: allow.sort,
            auto_filter: allow.auto_filter,
        };
    }
    ctx.workbook().set_sheet_protection(id, protection)?;
    Ok(Effect {
        result: serde_json::json!({
            "enabled": args.enable,
            "hash": hash,
        }),
        ..Effect::default()
    })
}

fn workbook_protect(
    ctx: &mut CommandContext<'_>,
    args: WorkbookProtectArgs,
) -> Result<Effect, CoreError> {
    let hash = args.password.as_deref().map(excel_xor_hash);
    ctx.workbook()
        .set_workbook_protection(omacell_core::workbook::WorkbookProtectionState {
            enabled: args.enable,
            lock_structure: args.enable && args.lock_structure,
            lock_windows: args.enable && args.lock_windows,
            password: hash.map(|value| format!("{value:04X}").into_bytes()),
        })?;
    Ok(Effect {
        result: serde_json::json!({
            "enabled": args.enable,
            "hash": hash,
        }),
        ..changed("workbook protection")
    })
}

fn sheet_protected_range(
    ctx: &mut CommandContext<'_>,
    args: ProtectedRangeArgs,
) -> Result<Effect, CoreError> {
    let sheet = match args.sheet.as_deref() {
        Some(name) => ctx
            .workbook_ref()
            .sheet_by_name(name)
            .map(|sheet| sheet.id)
            .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {name:?}")))?,
        None => ctx.workbook_ref().active_sheet(),
    };
    let mut protection = ctx
        .workbook_ref()
        .sheet(sheet)
        .map(|sheet| sheet.protection.clone())
        .ok_or_else(|| CoreError::sheet_id("unknown sheet"))?;
    protection
        .protected_ranges
        .retain(|range| !range.name.eq_ignore_ascii_case(&args.name));
    if !args.remove {
        if args.name.trim().is_empty() || args.ranges.is_empty() {
            return Err(crate::error::args(
                "protected range requires a name and at least one range",
            ));
        }
        let ranges = args
            .ranges
            .iter()
            .map(|value| {
                let range = resolve_range(ctx.workbook_ref(), value)?;
                if range.sheet != sheet {
                    return Err(crate::error::args(
                        "protected ranges must be on the target sheet",
                    ));
                }
                to_range(range)
            })
            .collect::<Result<Vec<_>, CoreError>>()?;
        protection.protected_ranges.push(ProtectedRange {
            name: args.name,
            ranges,
            password: args
                .password
                .as_deref()
                .map(excel_xor_hash)
                .map(|hash| format!("{hash:04X}").into_bytes()),
        });
    }
    ctx.workbook().set_sheet_protection(sheet, protection)?;
    Ok(changed("protected range"))
}

fn format_protection(
    ctx: &mut CommandContext<'_>,
    args: CellProtectionArgs,
) -> Result<Effect, CoreError> {
    if args.locked.is_none() && args.hidden.is_none() {
        return Err(crate::error::args(
            "format.protection requires locked and/or hidden",
        ));
    }
    let range = resolve_range(ctx.workbook_ref(), &args.range)?;
    let effect = patch_action_styles(
        ctx,
        range,
        |style, _, _| {
            if let Some(locked) = args.locked {
                style.protection.locked = locked;
            }
            if let Some(hidden) = args.hidden {
                style.protection.hidden = hidden;
            }
        },
        "cell protection",
    )?;
    for row in range.min_row..=range.max_row {
        for col in range.min_col..=range.max_col {
            let Some(mut slot) = ctx.workbook_ref().get(range.sheet, row, col)?.copied() else {
                continue;
            };
            if let Some(locked) = args.locked {
                slot.flags = slot
                    .flags
                    .with(omacell_core::storage::CellFlags::LOCKED, locked);
            }
            if let Some(hidden) = args.hidden {
                slot.flags = slot
                    .flags
                    .with(omacell_core::storage::CellFlags::HIDDEN, hidden);
            }
            ctx.workbook().set_slot(range.sheet, row, col, slot)?;
        }
    }
    Ok(effect)
}

fn sheet_reorder(
    ctx: &mut CommandContext<'_>,
    args: SheetReorderArgs,
) -> Result<Effect, CoreError> {
    let id = ctx
        .workbook_ref()
        .sheet_by_name(&args.sheet)
        .map(|s| s.id)
        .ok_or_else(|| CoreError::sheet_id("unknown sheet"))?;
    ctx.workbook().reorder_sheet(id, args.index as usize)?;
    Ok(changed("reorder"))
}

fn edit_texttocolumns(
    ctx: &mut CommandContext<'_>,
    args: TextToColumnsArgs,
) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    let columns = args
        .column_types
        .iter()
        .map(|kind| match kind.as_str() {
            "general" => Ok(TextColumnType::General),
            "text" => Ok(TextColumnType::Text),
            "skip" => Ok(TextColumnType::Skip),
            other => Err(crate::error::args(format!(
                "text-to-columns type must be general, text, or skip (got {other:?})"
            ))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mode = match args.mode.as_deref().unwrap_or("delimited") {
        "delimited" => TextToColumnsMode::Delimited {
            delimiters: args
                .delimiters
                .as_deref()
                .unwrap_or(&args.delim)
                .chars()
                .collect(),
        },
        "fixed" => TextToColumnsMode::Fixed {
            breaks: args.breaks,
        },
        other => {
            return Err(crate::error::args(format!(
                "text-to-columns mode must be delimited or fixed (got {other:?})"
            )));
        }
    };
    let n = text_to_columns_with_plan(
        ctx.workbook(),
        r.sheet,
        to_range(r)?,
        &TextToColumnsPlan { mode, columns },
    )?;
    Ok(Effect {
        result: serde_json::json!({"changed": n}),
        auto_recalc: true,
        ..Effect::default()
    })
}

fn range_removeduplicates(
    ctx: &mut CommandContext<'_>,
    args: RemoveDupArgs,
) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    let n = remove_duplicates_with_header(
        ctx.workbook(),
        r.sheet,
        to_range(r)?,
        &args.columns,
        args.has_headers,
    )?;
    Ok(Effect {
        result: serde_json::json!({"removed": n}),
        auto_recalc: true,
        ..Effect::default()
    })
}

fn range_consolidate(
    ctx: &mut CommandContext<'_>,
    args: ConsolidateArgs,
) -> Result<Effect, CoreError> {
    if args.sources.is_empty() {
        return Err(crate::error::args(
            "consolidate requires at least one source range",
        ));
    }
    let sources = args
        .sources
        .iter()
        .map(|source| {
            let range = resolve_range(ctx.workbook_ref(), source)?;
            Ok((range.sheet, to_range(range)?))
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    let dest = resolve_cell(ctx.workbook_ref(), &args.dest)?;
    let count = consolidate_by_position(
        ctx.workbook(),
        dest.sheet,
        CellRef::new(dest.row, dest.col)?,
        &sources,
    )?;
    Ok(Effect {
        result: serde_json::json!({"changed": count}),
        ..changed("consolidate")
    })
}

fn edit_clearcell(ctx: &mut CommandContext<'_>, args: RangeOnlyArgs) -> Result<Effect, CoreError> {
    let r = resolve_range(ctx.workbook_ref(), &args.range)?;
    let mut n = 0u32;
    for row in r.min_row..=r.max_row {
        for col in r.min_col..=r.max_col {
            ctx.workbook().set_cell_contents(r.sheet, row, col, "")?;
            n += 1;
        }
    }
    Ok(Effect {
        result: serde_json::json!({"changed": n}),
        auto_recalc: true,
        ..Effect::default()
    })
}

fn edit_clear(ctx: &mut CommandContext<'_>, args: EditClearArgs) -> Result<Effect, CoreError> {
    let range = resolve_range(ctx.workbook_ref(), &args.range)?;
    match args.what.as_str() {
        "contents" => edit_clearcell(ctx, RangeOnlyArgs { range: args.range }),
        "formats" => {
            let cells: Vec<_> = ctx
                .workbook_ref()
                .sheet(range.sheet)
                .ok_or_else(|| CoreError::sheet_id("unknown sheet"))?
                .store
                .iter_region(range.min_row, range.min_col, range.max_row, range.max_col)
                .map(|(row, col, _)| (row, col))
                .collect();
            for (row, col) in &cells {
                ctx.workbook().set_cell_style(
                    range.sheet,
                    *row,
                    *col,
                    omacell_core::style::Style::default(),
                )?;
                if let Some(mut slot) = ctx.workbook_ref().get(range.sheet, *row, *col)?.copied() {
                    slot.flags = slot
                        .flags
                        .with(omacell_core::storage::CellFlags::LOCKED, true)
                        .with(omacell_core::storage::CellFlags::HIDDEN, false);
                    ctx.workbook().set_slot(range.sheet, *row, *col, slot)?;
                }
            }
            Ok(Effect {
                result: serde_json::json!({"changed": cells.len()}),
                ..changed("clear formats")
            })
        }
        "comments" => clear_side_records(ctx, range, true, false),
        "hyperlinks" => clear_side_records(ctx, range, false, true),
        "all" => {
            let cells: Vec<_> = ctx
                .workbook_ref()
                .sheet(range.sheet)
                .ok_or_else(|| CoreError::sheet_id("unknown sheet"))?
                .store
                .iter_region(range.min_row, range.min_col, range.max_row, range.max_col)
                .map(|(row, col, _)| (row, col))
                .collect();
            for (row, col) in &cells {
                ctx.workbook().clear_cell(range.sheet, *row, *col)?;
            }
            let _ = clear_side_records(ctx, range, true, true)?;
            Ok(Effect {
                result: serde_json::json!({"changed": cells.len()}),
                ..changed("clear all")
            })
        }
        other => Err(crate::error::args(format!(
            "clear kind must be contents, formats, comments, hyperlinks, or all (got {other:?})"
        ))),
    }
}

fn edit_delete(ctx: &mut CommandContext<'_>, args: SelectionArgs) -> Result<Effect, CoreError> {
    edit_delete_common(ctx, args, false)
}

fn edit_change(ctx: &mut CommandContext<'_>, args: SelectionArgs) -> Result<Effect, CoreError> {
    edit_delete_common(ctx, args, true)
}

fn edit_delete_common(
    ctx: &mut CommandContext<'_>,
    args: SelectionArgs,
    begin_edit: bool,
) -> Result<Effect, CoreError> {
    let range = action_range(ctx, args.range.as_deref())?;
    let payload = copy_effect(ctx, range, false)?.result["payload"].clone();
    let mut changed = 0u64;
    for row in range.min_row..=range.max_row {
        for col in range.min_col..=range.max_col {
            ctx.workbook()
                .set_cell_contents(range.sheet, row, col, "")?;
            changed += 1;
        }
    }
    Ok(Effect {
        result: serde_json::json!({
            "changed": changed,
            "payload": payload,
            "begin_edit": begin_edit,
        }),
        auto_recalc: true,
        summary: ChangeSummary {
            cells: changed,
            text: if begin_edit { "change" } else { "delete" }.into(),
            ..ChangeSummary::default()
        },
        ..Effect::default()
    })
}

fn edit_clear_row(ctx: &mut CommandContext<'_>, args: SelectionArgs) -> Result<Effect, CoreError> {
    let range = action_range(ctx, args.range.as_deref())?;
    let cells: Vec<_> = ctx
        .workbook_ref()
        .sheet(range.sheet)
        .ok_or_else(|| CoreError::sheet_id("unknown sheet"))?
        .store
        .iter()
        .filter(|(row, _, _)| *row >= range.min_row && *row <= range.max_row)
        .map(|(row, col, _)| (row, col))
        .collect();
    for &(row, col) in &cells {
        ctx.workbook().clear_cell(range.sheet, row, col)?;
    }
    let whole_rows = crate::resolve::ResolvedRange {
        min_col: 0,
        max_col: omacell_core::limits::MAX_COLS - 1,
        ..range
    };
    let metadata = clear_side_records(ctx, whole_rows, true, true)?;
    let metadata_count = metadata.result["changed"].as_u64().unwrap_or(0);
    Ok(Effect {
        result: serde_json::json!({"changed": cells.len() as u64 + metadata_count}),
        auto_recalc: true,
        summary: ChangeSummary {
            cells: cells.len() as u64,
            text: "clear rows".into(),
            ..ChangeSummary::default()
        },
        ..Effect::default()
    })
}

fn edit_autosum(ctx: &mut CommandContext<'_>, args: SelectionArgs) -> Result<Effect, CoreError> {
    let range = action_range(ctx, args.range.as_deref())?;
    let mut changed = 0u64;
    for col in range.min_col..=range.max_col {
        let destination = range.max_row;
        let start = if range.min_row < destination {
            range.min_row
        } else {
            let mut row = destination;
            while row > 0 && ctx.workbook_ref().get(range.sheet, row - 1, col)?.is_some() {
                row -= 1;
            }
            row
        };
        let formula = if start < destination {
            let column = col_to_letters(col)?;
            format!("=SUM({column}{}:{column}{})", start + 1, destination)
        } else {
            "=SUM()".into()
        };
        ctx.workbook()
            .set_cell_contents(range.sheet, destination, col, &formula)?;
        changed += 1;
    }
    Ok(Effect {
        result: serde_json::json!({"changed": changed}),
        auto_recalc: true,
        summary: ChangeSummary {
            cells: changed,
            text: "autosum".into(),
            ..ChangeSummary::default()
        },
        ..Effect::default()
    })
}

fn edit_copy_formula_above(
    ctx: &mut CommandContext<'_>,
    args: SelectionArgs,
) -> Result<Effect, CoreError> {
    copy_above(ctx, args, true)
}

fn edit_copy_value_above(
    ctx: &mut CommandContext<'_>,
    args: SelectionArgs,
) -> Result<Effect, CoreError> {
    copy_above(ctx, args, false)
}

fn copy_above(
    ctx: &mut CommandContext<'_>,
    args: SelectionArgs,
    formulas: bool,
) -> Result<Effect, CoreError> {
    let range = action_range(ctx, args.range.as_deref())?;
    if range.min_row == 0 {
        return Err(crate::error::args("the first row has no row above it"));
    }
    let sources = (range.min_col..=range.max_col)
        .map(|col| {
            let source = CellRef::new(range.min_row - 1, col)?;
            Ok((
                col,
                copy_range(
                    ctx.workbook_ref(),
                    range.sheet,
                    RangeRef::from_corners(source, source),
                ),
            ))
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    let spec = PasteSpecial {
        values: !formulas,
        formulas,
        ..PasteSpecial::default()
    };
    let mut changed = 0u32;
    for row in range.min_row..=range.max_row {
        for (col, source) in &sources {
            changed += paste_special_from(
                ctx.workbook(),
                range.sheet,
                CellRef::new(row, *col)?,
                source,
                spec,
                Some((range.min_row - 1, *col)),
                Some(range.sheet),
            )?;
        }
    }
    Ok(Effect {
        result: serde_json::json!({"changed": changed}),
        auto_recalc: true,
        summary: ChangeSummary {
            cells: u64::from(changed),
            text: if formulas {
                "copy formula above"
            } else {
                "copy value above"
            }
            .into(),
            ..ChangeSummary::default()
        },
        ..Effect::default()
    })
}

fn edit_insert_date(
    ctx: &mut CommandContext<'_>,
    args: InsertSerialArgs,
) -> Result<Effect, CoreError> {
    insert_serial(ctx, args, true)
}

fn edit_insert_time(
    ctx: &mut CommandContext<'_>,
    args: InsertSerialArgs,
) -> Result<Effect, CoreError> {
    insert_serial(ctx, args, false)
}

fn insert_serial(
    ctx: &mut CommandContext<'_>,
    args: InsertSerialArgs,
    date: bool,
) -> Result<Effect, CoreError> {
    let range = action_range(ctx, args.range.as_deref())?;
    let serial = match args.serial {
        Some(value) if value.is_finite() => value,
        Some(_) => return Err(crate::error::args("Excel serial must be finite")),
        None if ctx.origin() == omacell_core::command::Origin::User => {
            current_excel_serial(ctx.workbook_ref().settings().date_system)?
        }
        None => {
            return Err(crate::error::args(
                "automated date/time insertion requires an explicit serial",
            ));
        }
    };
    let value = if date {
        serial.floor()
    } else {
        serial.rem_euclid(1.0)
    };
    let format = ctx
        .workbook()
        .intern_num_fmt(if date { "m/d/yyyy" } else { "h:mm:ss" })?;
    let mut changed = 0u64;
    for row in range.min_row..=range.max_row {
        for col in range.min_col..=range.max_col {
            ctx.workbook().set_number(range.sheet, row, col, value)?;
            let mut style = ctx
                .workbook_ref()
                .get(range.sheet, row, col)?
                .and_then(|slot| ctx.workbook_ref().intern().styles.get(slot.style))
                .cloned()
                .unwrap_or_default();
            style.num_fmt = format;
            ctx.workbook()
                .set_cell_style(range.sheet, row, col, style)?;
            changed += 1;
        }
    }
    Ok(Effect {
        result: serde_json::json!({"changed": changed, "serial": value}),
        auto_recalc: true,
        summary: ChangeSummary {
            cells: changed,
            styles: changed,
            text: if date { "insert date" } else { "insert time" }.into(),
            ..ChangeSummary::default()
        },
        ..Effect::default()
    })
}

fn current_excel_serial(system: omacell_core::workbook::DateSystem) -> Result<f64, CoreError> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| crate::error::args("system clock is before the Unix epoch"))?
        .as_secs_f64();
    let epoch = match system {
        omacell_core::workbook::DateSystem::Excel1900 => 25_569.0,
        omacell_core::workbook::DateSystem::Excel1904 => 24_107.0,
    };
    Ok(epoch + seconds / 86_400.0)
}

fn edit_repeat_placeholder(
    _ctx: &mut CommandContext<'_>,
    _args: RepeatArgs,
) -> Result<Effect, CoreError> {
    Err(crate::error::args(
        "edit.repeat is dispatched by the session and has no prior command",
    ))
}

fn clear_side_records(
    ctx: &mut CommandContext<'_>,
    range: crate::resolve::ResolvedRange,
    comments: bool,
    hyperlinks: bool,
) -> Result<Effect, CoreError> {
    let sheet = ctx
        .workbook_ref()
        .sheet(range.sheet)
        .ok_or_else(|| CoreError::sheet_id("unknown sheet"))?;
    let notes: Vec<_> = if comments {
        sheet
            .notes
            .keys()
            .filter(|&&(row, col)| {
                row >= range.min_row
                    && row <= range.max_row
                    && col >= range.min_col
                    && col <= range.max_col
            })
            .copied()
            .collect()
    } else {
        Vec::new()
    };
    let threads: Vec<_> = if comments {
        sheet
            .comments
            .keys()
            .filter(|&&(row, col)| {
                row >= range.min_row
                    && row <= range.max_row
                    && col >= range.min_col
                    && col <= range.max_col
            })
            .copied()
            .collect()
    } else {
        Vec::new()
    };
    let links: Vec<_> = if hyperlinks {
        sheet
            .hyperlinks
            .keys()
            .filter(|&&(row, col)| {
                row >= range.min_row
                    && row <= range.max_row
                    && col >= range.min_col
                    && col <= range.max_col
            })
            .copied()
            .collect()
    } else {
        Vec::new()
    };
    for (row, col) in &notes {
        ctx.workbook().set_note(range.sheet, *row, *col, None)?;
    }
    for (row, col) in &threads {
        ctx.workbook().set_comment(range.sheet, *row, *col, None)?;
    }
    for (row, col) in &links {
        ctx.workbook()
            .set_hyperlink(range.sheet, *row, *col, None)?;
    }
    Ok(Effect {
        result: serde_json::json!({
            "changed": notes.len() + threads.len() + links.len()
        }),
        ..changed("clear metadata")
    })
}

fn action_range(
    ctx: &CommandContext<'_>,
    range: Option<&str>,
) -> Result<crate::resolve::ResolvedRange, CoreError> {
    if let Some(range) = range {
        return resolve_range(ctx.workbook_ref(), range);
    }
    let sheet = ctx.workbook_ref().active_sheet();
    let selection = ctx
        .workbook_ref()
        .sheet(sheet)
        .ok_or_else(|| CoreError::sheet_id("active sheet is missing"))?
        .view
        .selection;
    Ok(crate::resolve::ResolvedRange {
        sheet,
        min_row: selection.start.row.min(selection.end.row),
        min_col: selection.start.col.min(selection.end.col),
        max_row: selection.start.row.max(selection.end.row),
        max_col: selection.start.col.max(selection.end.col),
    })
}

fn first_style(
    ctx: &CommandContext<'_>,
    range: crate::resolve::ResolvedRange,
) -> omacell_core::style::Style {
    ctx.workbook_ref()
        .get(range.sheet, range.min_row, range.min_col)
        .ok()
        .flatten()
        .and_then(|slot| ctx.workbook_ref().intern().styles.get(slot.style))
        .cloned()
        .unwrap_or_default()
}

fn patch_action_styles(
    ctx: &mut CommandContext<'_>,
    range: crate::resolve::ResolvedRange,
    mut patch: impl FnMut(&mut omacell_core::style::Style, u32, u16),
    label: &str,
) -> Result<Effect, CoreError> {
    let mut count = 0u64;
    for row in range.min_row..=range.max_row {
        for col in range.min_col..=range.max_col {
            let mut style = ctx
                .workbook_ref()
                .get(range.sheet, row, col)?
                .and_then(|slot| ctx.workbook_ref().intern().styles.get(slot.style))
                .cloned()
                .unwrap_or_default();
            patch(&mut style, row, col);
            ctx.workbook()
                .set_cell_style(range.sheet, row, col, style)?;
            count += 1;
        }
    }
    Ok(Effect {
        result: serde_json::json!({"changed": count}),
        summary: ChangeSummary {
            cells: count,
            styles: count,
            text: label.into(),
            ..ChangeSummary::default()
        },
        ..Effect::default()
    })
}

fn format_bold(ctx: &mut CommandContext<'_>, args: FormatActionArgs) -> Result<Effect, CoreError> {
    let range = action_range(ctx, args.range.as_deref())?;
    let enabled = args.enabled.unwrap_or(!first_style(ctx, range).font.bold);
    patch_action_styles(ctx, range, |style, _, _| style.font.bold = enabled, "bold")
}

fn format_italic(
    ctx: &mut CommandContext<'_>,
    args: FormatActionArgs,
) -> Result<Effect, CoreError> {
    let range = action_range(ctx, args.range.as_deref())?;
    let enabled = args.enabled.unwrap_or(!first_style(ctx, range).font.italic);
    patch_action_styles(
        ctx,
        range,
        |style, _, _| style.font.italic = enabled,
        "italic",
    )
}

fn format_underline(
    ctx: &mut CommandContext<'_>,
    args: FormatActionArgs,
) -> Result<Effect, CoreError> {
    let range = action_range(ctx, args.range.as_deref())?;
    let enabled = args.enabled.unwrap_or(matches!(
        first_style(ctx, range).font.underline,
        omacell_core::style::Underline::None
    ));
    patch_action_styles(
        ctx,
        range,
        |style, _, _| {
            style.font.underline = if enabled {
                omacell_core::style::Underline::Single
            } else {
                omacell_core::style::Underline::None
            };
        },
        "underline",
    )
}

fn format_indent(
    ctx: &mut CommandContext<'_>,
    args: FormatActionArgs,
) -> Result<Effect, CoreError> {
    let range = action_range(ctx, args.range.as_deref())?;
    patch_action_styles(
        ctx,
        range,
        |style, _, _| style.alignment.indent = style.alignment.indent.saturating_add(1),
        "indent",
    )
}

fn format_outdent(
    ctx: &mut CommandContext<'_>,
    args: FormatActionArgs,
) -> Result<Effect, CoreError> {
    let range = action_range(ctx, args.range.as_deref())?;
    patch_action_styles(
        ctx,
        range,
        |style, _, _| style.alignment.indent = style.alignment.indent.saturating_sub(1),
        "outdent",
    )
}

fn apply_named_format(
    ctx: &mut CommandContext<'_>,
    args: FormatActionArgs,
    code: &str,
    label: &str,
) -> Result<Effect, CoreError> {
    let range = action_range(ctx, args.range.as_deref())?;
    let format = ctx.workbook().intern_num_fmt(code)?;
    patch_action_styles(ctx, range, |style, _, _| style.num_fmt = format, label)
}

fn format_general(
    ctx: &mut CommandContext<'_>,
    args: FormatActionArgs,
) -> Result<Effect, CoreError> {
    apply_named_format(ctx, args, "General", "general format")
}

fn format_numberstyle(
    ctx: &mut CommandContext<'_>,
    args: FormatActionArgs,
) -> Result<Effect, CoreError> {
    apply_named_format(ctx, args, "#,##0.00", "number format")
}

fn format_time(ctx: &mut CommandContext<'_>, args: FormatActionArgs) -> Result<Effect, CoreError> {
    apply_named_format(ctx, args, "h:mm:ss", "time format")
}

fn format_date(ctx: &mut CommandContext<'_>, args: FormatActionArgs) -> Result<Effect, CoreError> {
    apply_named_format(ctx, args, "m/d/yyyy", "date format")
}

fn format_currency(
    ctx: &mut CommandContext<'_>,
    args: FormatActionArgs,
) -> Result<Effect, CoreError> {
    apply_named_format(ctx, args, "$#,##0.00", "currency format")
}

fn format_percent(
    ctx: &mut CommandContext<'_>,
    args: FormatActionArgs,
) -> Result<Effect, CoreError> {
    apply_named_format(ctx, args, "0.00%", "percent format")
}

fn format_scientific(
    ctx: &mut CommandContext<'_>,
    args: FormatActionArgs,
) -> Result<Effect, CoreError> {
    apply_named_format(ctx, args, "0.00E+00", "scientific format")
}

fn format_border_outline(
    ctx: &mut CommandContext<'_>,
    args: FormatActionArgs,
) -> Result<Effect, CoreError> {
    let range = action_range(ctx, args.range.as_deref())?;
    let side = omacell_core::style::BorderSide {
        style: omacell_core::style::BorderStyle::Thin,
        color: omacell_core::style::Color::Auto,
    };
    patch_action_styles(
        ctx,
        range,
        |style, row, col| {
            if row == range.min_row {
                style.border.top = side;
            }
            if row == range.max_row {
                style.border.bottom = side;
            }
            if col == range.min_col {
                style.border.left = side;
            }
            if col == range.max_col {
                style.border.right = side;
            }
        },
        "outline border",
    )
}

fn format_border_none(
    ctx: &mut CommandContext<'_>,
    args: FormatActionArgs,
) -> Result<Effect, CoreError> {
    let range = action_range(ctx, args.range.as_deref())?;
    patch_action_styles(
        ctx,
        range,
        |style, _, _| style.border = omacell_core::style::Border::default(),
        "remove borders",
    )
}

fn format_panel(ctx: &mut CommandContext<'_>, args: FormatActionArgs) -> Result<Effect, CoreError> {
    let range = action_range(ctx, args.range.as_deref())?;
    let style = first_style(ctx, range);
    let format = ctx
        .workbook_ref()
        .num_fmt_code(style.num_fmt)
        .map(|value| value.into_owned());
    Ok(Effect::query(serde_json::json!({
        "range": args.range,
        "style": style,
        "number_format": format,
    })))
}

fn changed(text: &str) -> Effect {
    Effect {
        result: serde_json::json!({"ok": true}),
        auto_recalc: true,
        rebuild: true,
        summary: ChangeSummary {
            text: text.into(),
            ..ChangeSummary::default()
        },
        inverse: Vec::new(),
        ..Effect::default()
    }
}
