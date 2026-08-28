//! Typed command arguments. Unknown fields are rejected.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `cell.set`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CellSetArgs {
    /// A1 cell, optionally sheet-qualified.
    #[serde(rename = "ref")]
    pub cell_ref: String,
    /// Formula-bar text.
    pub input: String,
}

/// `cell.clear`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CellClearArgs {
    /// A1 cell, optionally sheet-qualified.
    #[serde(rename = "ref")]
    pub cell_ref: String,
}

/// `range.set`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RangeSetArgs {
    /// A1 range.
    pub range: String,
    /// Fill every cell with this formula-bar text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// Row-major formula-bar values. `null` clears a cell's contents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<Vec<Option<String>>>>,
}

/// `range.clear`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RangeClearArgs {
    /// A1 range.
    pub range: String,
}

/// `sheet.add`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SheetAddArgs {
    /// Sheet name. Generated as `SheetN` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// `sheet.rename`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SheetRenameArgs {
    /// Current sheet name.
    pub sheet: String,
    /// New name.
    pub name: String,
}

/// `sheet.visibility`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SheetVisibilityArgs {
    /// Sheet name.
    pub sheet: String,
    /// `visible`, `hidden`, or `very_hidden`.
    pub visibility: String,
}

/// `sheet.remove` (internal).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SheetRemoveArgs {
    /// Sheet name.
    pub sheet: String,
}

/// `name.define`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NameDefineArgs {
    /// Defined name.
    pub name: String,
    /// Sheet-local scope. Omitted = workbook.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,
    /// Referent.
    pub referent: NameReferentArg,
    /// Optional comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// Defined-name referent.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum NameReferentArg {
    /// A1 range.
    Range {
        /// Range text.
        range: String,
    },
    /// Constant JSON value (number, bool, string, or null).
    Constant {
        /// Value.
        value: serde_json::Value,
    },
    /// Formula text.
    Formula {
        /// Formula source.
        formula: String,
    },
}

/// `name.remove`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NameRemoveArgs {
    /// Defined name.
    pub name: String,
    /// Sheet-local scope. Omitted = workbook.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,
}

/// `format.number`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FormatNumberArgs {
    /// A1 range.
    pub range: String,
    /// Excel format code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Built-in or interned `numFmtId`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_fmt_id: Option<u32>,
}

/// `style.set`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StyleSetArgs {
    /// A1 range.
    pub range: String,
    /// Bold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    /// Italic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    /// Underline on/off (single).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
    /// Strikethrough.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strike: Option<bool>,
    /// Font size in points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_pt: Option<f64>,
    /// Typeface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_name: Option<String>,
    /// Font colour as `0xAARRGGBB`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_color_argb: Option<u32>,
    /// Solid fill as `0xAARRGGBB`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_argb: Option<u32>,
    /// Wrap text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap: Option<bool>,
    /// Horizontal alignment name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizontal: Option<String>,
    /// Vertical alignment name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertical: Option<String>,
    /// Protection locked flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    /// Hide formula when protected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    /// Number format code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

/// `calc.recalc`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CalcRecalcArgs {
    /// `incremental`, `full`, or `rebuild`. Default `full` so Manual mode still calculates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// `calc.mode`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CalcModeArgs {
    /// `automatic`, `automatic_except_tables`, or `manual`.
    pub mode: String,
}

/// `undo` / `redo`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmptyArgs {}

/// Internal `cell.restore`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CellRestoreArgs {
    /// A1 cell.
    #[serde(rename = "ref")]
    pub cell_ref: String,
    /// Drop the slot entirely (including style).
    #[serde(default)]
    pub absent: bool,
    /// Formula source when the slot remains.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
    /// Interner-independent encoded cached or literal value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    /// Style record (serde of [`omacell_core::style::Style`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<serde_json::Value>,
    /// Custom number format code paired with `style`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Packed [`omacell_core::storage::CellFlags`] bits.
    #[serde(default)]
    pub flags: u8,
}

/// Internal `style.restore`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StyleRestoreArgs {
    /// A1 cell.
    #[serde(rename = "ref")]
    pub cell_ref: String,
    /// Style record.
    pub style: serde_json::Value,
    /// Custom number format code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Drop the slot if it did not exist before a style-only write.
    #[serde(default)]
    pub absent: bool,
}
