//! Find / replace / go-to models (F-5.8).

/// Search scope.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FindScope {
    /// Active sheet.
    #[default]
    Sheet,
    /// Whole workbook.
    Workbook,
}

/// Find/replace panel state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindReplace {
    /// Needle.
    pub find: String,
    /// Replacement.
    pub replace: String,
    /// Values vs formulas.
    pub in_formulas: bool,
    /// Whole cell.
    pub whole_cell: bool,
    /// Case sensitive.
    pub case: bool,
    /// Regex (extension).
    pub regex: bool,
    /// Scope.
    pub scope: FindScope,
    /// Last preview count.
    pub preview: u32,
}

/// Go To target.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GoTo {
    /// Address or name.
    pub target: String,
}
