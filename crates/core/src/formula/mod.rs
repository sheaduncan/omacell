//! Excel formula lexer, parser, printer, rewrite, and deps (WP-03).
//!
//! The parser accepts the canonical locale only (`,` argument separator, `.`
//! decimal). Localized entry is converted at the editor boundary (WP-14).
//! Function names are any identifier followed by `(`; the registry is WP-05.

mod ast;
mod deps;
mod error;
mod lexer;
mod parser;
mod printer;
mod rewrite;

pub use ast::{
    BinOp, Callee, Expr, ExprKind, PostfixOp, PrefixOp, Span, StructuredRef, TableColumns,
    TableItem,
};
pub use deps::{DYNAMIC_FUNCS, Deps, VOLATILE_FUNCS, collect_deps};
pub use error::{ParseError, codes};
pub use parser::{parse, parse_editor, parse_editor_with, parse_with};
pub use printer::{print, print_expr, print_with};
pub(crate) use rewrite::invalidate_deleted_references;
pub use rewrite::{
    RewriteOp, adjust_cols, adjust_rows, apply, copy_delta, move_range, rename_sheet, rename_table,
    rewrite_print,
};

/// Maximum nested function levels supported by Excel.
pub const MAX_FORMULA_DEPTH: u32 = 64;

/// A1 vs R1C1 reference style.
///
/// ```
/// use omacell_core::formula::RefStyle;
/// assert_eq!(RefStyle::A1, RefStyle::default());
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RefStyle {
    /// `A1`, `$B$2`, `A:A`.
    #[default]
    A1,
    /// `R1C1`, `R[-1]C[2]`. Relatives resolve against [`ParseOptions::base_row`].
    R1C1,
}

/// Options for [`parse_with`].
///
/// ```
/// use omacell_core::formula::{parse_with, ParseOptions, RefStyle};
/// let f = parse_with("=R[1]C[1]", ParseOptions {
///     style: RefStyle::R1C1,
///     ..ParseOptions::default()
/// }).unwrap();
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseOptions {
    /// A1 or R1C1.
    pub style: RefStyle,
    /// 0-based base row for R1C1 relatives.
    pub base_row: u32,
    /// 0-based base column for R1C1 relatives.
    pub base_col: u16,
    /// When true, the parser keeps a partial AST on error (editor mode).
    pub lenient: bool,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            style: RefStyle::A1,
            base_row: 0,
            base_col: 0,
            lenient: false,
        }
    }
}

/// Options for [`print_with`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrintOptions {
    /// A1 or R1C1 output.
    pub style: RefStyle,
    /// R1C1 base row.
    pub base_row: u32,
    /// R1C1 base column.
    pub base_col: u16,
}

impl Default for PrintOptions {
    fn default() -> Self {
        Self {
            style: RefStyle::A1,
            base_row: 0,
            base_col: 0,
        }
    }
}

/// Parsed formula: AST plus the style/base used to parse it.
///
/// ```
/// use omacell_core::formula::{parse, print};
/// let f = parse("=sum(a1)").unwrap();
/// assert_eq!(print(&f), "=SUM(A1)");
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Formula {
    /// Root expression.
    pub ast: Expr,
    /// Style used while parsing (and default print style).
    pub style: RefStyle,
    /// R1C1 base row.
    pub base_row: u32,
    /// R1C1 base column.
    pub base_col: u16,
}

/// Result of [`parse_editor`]: a partial tree plus the first error.
#[derive(Clone, Debug, PartialEq)]
pub struct PartialParse {
    /// Tree recovered so far (may be `None` on total failure).
    pub expr: Option<Expr>,
    /// First error, if any.
    pub error: Option<ParseError>,
}
