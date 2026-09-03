//! Formula AST. References keep relative/absolute flags on [`CellRef`].

use crate::addr::{CellRef, RangeRef, SheetSpec};
use crate::error::ErrorKind;

/// UTF-8 byte span in the original formula source.
///
/// ```
/// use omacell_core::formula::Span;
/// let s = Span { start: 0, end: 2 };
/// assert_eq!(s.len(), 2);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    /// Inclusive start byte.
    pub start: u32,
    /// Exclusive end byte.
    pub end: u32,
}

impl Span {
    /// Build a span from `usize` offsets.
    #[must_use]
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            start: start as u32,
            end: end as u32,
        }
    }

    /// Byte length.
    #[must_use]
    pub fn len(self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    /// Whether the span is empty.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.end <= self.start
    }

    /// Cover `self` and `other`.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

/// Prefix operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PrefixOp {
    /// Unary `+`.
    Plus,
    /// Unary `-`.
    Minus,
    /// Implicit intersection `@`.
    ImplicitIntersect,
}

/// Postfix operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PostfixOp {
    /// Percent `%` (value × 0.01 at eval).
    Percent,
    /// Spill `#`.
    Spill,
}

/// Infix operators (Excel F-3.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinOp {
    /// Range `:`.
    Range,
    /// Intersection (whitespace).
    Isect,
    /// Union `,`.
    Union,
    /// `^`.
    Pow,
    /// `*`.
    Mul,
    /// `/`.
    Div,
    /// `+`.
    Add,
    /// `-`.
    Sub,
    /// `&`.
    Concat,
    /// `=`.
    Eq,
    /// `<>`.
    Ne,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
}

impl BinOp {
    /// Canonical source glyph.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Range => ":",
            Self::Isect => " ",
            Self::Union => ",",
            Self::Pow => "^",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Add => "+",
            Self::Sub => "-",
            Self::Concat => "&",
            Self::Eq => "=",
            Self::Ne => "<>",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
        }
    }
}

/// How a function is invoked.
#[derive(Clone, Debug, PartialEq)]
pub enum Callee {
    /// `SUM` / `LAMBDA` / any identifier followed by `(`.
    Name(String),
    /// Immediately-invoked expression: `LAMBDA(x,x)(1)`.
    Expr(Box<Expr>),
}

/// Structured-table specifier (`#All`, `#This Row`, …).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TableItem {
    /// `#All`.
    All,
    /// `#Data`.
    Data,
    /// `#Headers`.
    Headers,
    /// `#Totals`.
    Totals,
    /// `#This Row`.
    ThisRow,
}

impl TableItem {
    /// Canonical specifier text including `#`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "#All",
            Self::Data => "#Data",
            Self::Headers => "#Headers",
            Self::Totals => "#Totals",
            Self::ThisRow => "#This Row",
        }
    }

    pub(crate) fn parse(s: &str) -> Option<Self> {
        let t = s.trim();
        if t.eq_ignore_ascii_case("#All") {
            Some(Self::All)
        } else if t.eq_ignore_ascii_case("#Data") {
            Some(Self::Data)
        } else if t.eq_ignore_ascii_case("#Headers") {
            Some(Self::Headers)
        } else if t.eq_ignore_ascii_case("#Totals") {
            Some(Self::Totals)
        } else if t.eq_ignore_ascii_case("#This Row") {
            Some(Self::ThisRow)
        } else {
            None
        }
    }
}

/// Column selector inside a structured reference.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TableColumns {
    /// `[Col]` or `[[Col]]`.
    One(String),
    /// `[Col1]:[Col2]`.
    Span {
        /// Left column.
        start: String,
        /// Right column.
        end: String,
    },
}

/// Parsed structured reference (`Table[[#Headers],[Col]]`, `[@Col]`).
///
/// ```
/// use omacell_core::formula::{parse, ExprKind};
/// let f = parse("=Sales[Amount]").unwrap();
/// assert!(matches!(f.ast.kind, ExprKind::Structured(_)));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StructuredRef {
    /// Table name; `None` for unqualified `[Col]` / `[@Amt]`.
    pub table: Option<String>,
    /// `#All` / `#Data` / … when present.
    pub item: Option<TableItem>,
    /// This-row (`@` / `[#This Row]`) shorthand.
    pub this_row: bool,
    /// Column or column span.
    pub columns: Option<TableColumns>,
    /// Inner `[...]` text as written (minus the table name) for stable print.
    pub inner: String,
}

/// A formula expression with a source span.
///
/// ```
/// use omacell_core::formula::{parse, ExprKind};
/// let f = parse("=A1+1").unwrap();
/// assert!(matches!(f.ast.kind, ExprKind::Binary { .. }));
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    /// Node kind.
    pub kind: ExprKind,
    /// Byte span covering this node in the source.
    pub span: Span,
}

impl Expr {
    pub(crate) fn new(kind: ExprKind, span: Span) -> Self {
        Self { kind, span }
    }

    /// Walk every node, children first.
    pub fn walk<F: FnMut(&Expr)>(&self, visit: &mut F) {
        match &self.kind {
            ExprKind::Array(rows) => {
                for row in rows {
                    for c in row {
                        c.walk(visit);
                    }
                }
            }
            ExprKind::ThreeD { inner, .. }
            | ExprKind::External { inner, .. }
            | ExprKind::Prefix { expr: inner, .. }
            | ExprKind::Postfix { expr: inner, .. }
            | ExprKind::Paren(inner) => inner.walk(visit),
            ExprKind::Binary { left, right, .. } => {
                left.walk(visit);
                right.walk(visit);
            }
            ExprKind::Call { callee, args } => {
                if let Callee::Expr(e) = callee {
                    e.walk(visit);
                }
                for a in args.iter().flatten() {
                    a.walk(visit);
                }
            }
            ExprKind::Number(_)
            | ExprKind::String(_)
            | ExprKind::Bool(_)
            | ExprKind::Error(_)
            | ExprKind::Cell { .. }
            | ExprKind::Range { .. }
            | ExprKind::Name { .. }
            | ExprKind::Structured(_) => {}
        }
        visit(self);
    }

    /// Reconstruct with a mapped kind (children first).
    pub fn map<F: FnMut(Expr) -> Expr>(self, f: &mut F) -> Expr {
        self.map_inner(f, true)
    }

    pub(crate) fn map_local<F: FnMut(Expr) -> Expr>(self, f: &mut F) -> Expr {
        self.map_inner(f, false)
    }

    fn map_inner<F: FnMut(Expr) -> Expr>(self, f: &mut F, traverse_external: bool) -> Expr {
        let mapped = match self.kind {
            ExprKind::Array(rows) => ExprKind::Array(
                rows.into_iter()
                    .map(|row| {
                        row.into_iter()
                            .map(|c| c.map_inner(f, traverse_external))
                            .collect()
                    })
                    .collect(),
            ),
            ExprKind::ThreeD { sheets, inner } => ExprKind::ThreeD {
                sheets,
                inner: Box::new(inner.map_inner(f, traverse_external)),
            },
            ExprKind::External {
                workbook,
                inner,
                quoted,
            } => ExprKind::External {
                workbook,
                inner: if traverse_external {
                    Box::new(inner.map_inner(f, true))
                } else {
                    inner
                },
                quoted,
            },
            ExprKind::Prefix { op, expr } => ExprKind::Prefix {
                op,
                expr: Box::new(expr.map_inner(f, traverse_external)),
            },
            ExprKind::Postfix { expr, op } => ExprKind::Postfix {
                expr: Box::new(expr.map_inner(f, traverse_external)),
                op,
            },
            ExprKind::Binary { op, left, right } => ExprKind::Binary {
                op,
                left: Box::new(left.map_inner(f, traverse_external)),
                right: Box::new(right.map_inner(f, traverse_external)),
            },
            ExprKind::Paren(inner) => {
                ExprKind::Paren(Box::new(inner.map_inner(f, traverse_external)))
            }
            ExprKind::Call { callee, args } => {
                let callee = match callee {
                    Callee::Name(n) => Callee::Name(n),
                    Callee::Expr(e) => Callee::Expr(Box::new(e.map_inner(f, traverse_external))),
                };
                let args = args
                    .into_iter()
                    .map(|a| a.map(|e| e.map_inner(f, traverse_external)))
                    .collect();
                ExprKind::Call { callee, args }
            }
            other => other,
        };
        f(Expr {
            kind: mapped,
            span: self.span,
        })
    }
}

/// Formula node kinds. Named, structured, spill, and 3-D refs are distinct.
#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    /// Numeric literal.
    Number(f64),
    /// `"..."` with `""` unescaped.
    String(String),
    /// `TRUE` / `FALSE`.
    Bool(bool),
    /// Error literal (`#N/A`, `#REF!`, …).
    Error(ErrorKind),
    /// `{1,2;3,4}` — rows of columns of scalar expressions.
    Array(Vec<Vec<Expr>>),
    /// Single cell, optional unresolved sheet.
    Cell {
        /// Sheet qualifier (`Sheet1!`).
        sheet: Option<SheetSpec>,
        /// Grid address with abs flags.
        cell: CellRef,
    },
    /// `A1:B2`, `A:A`, `1:1`.
    Range {
        /// Sheet qualifier.
        sheet: Option<SheetSpec>,
        /// Corners and whole-row/column flags.
        range: RangeRef,
    },
    /// `Sheet1:Sheet3!A1` (distinct from a 2-D range).
    ThreeD {
        /// Start and end sheet names (`end` is `Some`).
        sheets: SheetSpec,
        /// Body (cell, range, or name).
        inner: Box<Expr>,
    },
    /// Defined name (`Revenue`, `Sheet1!TaxRate`).
    Name {
        /// Optional sheet qualifier.
        sheet: Option<SheetSpec>,
        /// Name spelling as written.
        name: String,
    },
    /// Structured table reference.
    Structured(StructuredRef),
    /// `[Book.xlsx]Sheet1!A1` (F-1.5).
    External {
        /// Workbook file name or indexed `[1]`.
        workbook: String,
        /// Sheet-qualified inner ref.
        inner: Box<Expr>,
        /// True when the source used `'[book]sheet'!`.
        quoted: bool,
    },
    /// Prefix `+` / `-` / `@`.
    Prefix {
        /// Operator.
        op: PrefixOp,
        /// Operand.
        expr: Box<Expr>,
    },
    /// Postfix `%` / `#`.
    Postfix {
        /// Operand.
        expr: Box<Expr>,
        /// Operator.
        op: PostfixOp,
    },
    /// Infix operator.
    Binary {
        /// Operator.
        op: BinOp,
        /// Left operand.
        left: Box<Expr>,
        /// Right operand.
        right: Box<Expr>,
    },
    /// `(expr)` — kept so grouping round-trips.
    Paren(Box<Expr>),
    /// Function or IIFE call. `None` args are omitted (`INDEX(A1:C9,,2)`).
    Call {
        /// Callee.
        callee: Callee,
        /// Arguments; `None` = omitted.
        args: Vec<Option<Expr>>,
    },
}
