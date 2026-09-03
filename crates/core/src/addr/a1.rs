//! A1 parse and print (`$`, sheet names, whole-row/column, 3-D).

use crate::error::CoreError;
use crate::limits::{MAX_COLS, MAX_ROWS};

use super::letters::{col_from_letters, col_to_letters};
use super::scan::Cursor;
use super::{CellRef, ParsedRef, RangeRef, RefKind, SheetSpec, sheet_name_needs_quote};

/// Parse an A1 cell, range, whole-row/column, or 3-D reference.
///
/// ```
/// use omacell_core::addr::{parse_a1, RefKind};
/// let parsed = parse_a1("$A$1").unwrap();
/// assert!(matches!(parsed.kind, RefKind::Cell(_)));
/// assert_eq!(parsed.to_a1(), "$A$1");
/// ```
pub fn parse_a1(input: &str) -> Result<ParsedRef, CoreError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(CoreError::addr_parse("empty address"));
    }
    let (sheet, body) = split_sheet(input)?;
    let kind = parse_a1_body(body)?;
    Ok(ParsedRef { sheet, kind })
}

/// Parse a single A1 cell (`A1`, `$B$2`). Ranges are rejected.
///
/// ```
/// use omacell_core::addr::parse_a1_cell;
/// let cell = parse_a1_cell("C3").unwrap();
/// assert_eq!((cell.row, cell.col), (2, 2));
/// ```
pub fn parse_a1_cell(input: &str) -> Result<CellRef, CoreError> {
    let parsed = parse_a1(input)?;
    if parsed.sheet.is_some() {
        return Err(CoreError::addr_parse(
            "cell-only parser does not accept a sheet qualifier; use parse_a1",
        ));
    }
    match parsed.kind {
        RefKind::Cell(cell) => Ok(cell),
        RefKind::Range(_) => Err(CoreError::addr_parse(
            "expected a cell address, got a range",
        )),
    }
}

pub(super) fn parse_a1_body(body: &str) -> Result<RefKind, CoreError> {
    if body.is_empty() {
        return Err(CoreError::addr_parse("empty address body"));
    }
    match body.split_once(':') {
        Some((left, right)) => {
            let a = parse_a1_item(left)?;
            let b = parse_a1_item(right)?;
            Ok(RefKind::Range(range_from_items(a, b)?))
        }
        None => match parse_a1_item(body)? {
            A1Item::Cell(cell) => Ok(RefKind::Cell(cell)),
            other => Ok(RefKind::Range(range_from_items(other, other)?)),
        },
    }
}

#[derive(Clone, Copy)]
enum A1Item {
    Cell(CellRef),
    WholeCol { col: u16, abs: bool },
    WholeRow { row: u32, abs: bool },
}

fn parse_a1_item(s: &str) -> Result<A1Item, CoreError> {
    let mut cur = Cursor::new(s);
    let first_abs = cur.eat_char('$');
    let letters = cur.eat_while(|ch| ch.is_ascii_alphabetic());
    if !letters.is_empty() {
        let col = col_from_letters(letters)?;
        if cur.is_empty() {
            return Ok(A1Item::WholeCol {
                col,
                abs: first_abs,
            });
        }
        let row_abs = cur.eat_char('$');
        let digits = cur.eat_while(|ch| ch.is_ascii_digit());
        if digits.is_empty() || !cur.is_empty() {
            return Err(CoreError::addr_parse(format!("invalid A1 item {s:?}")));
        }
        let row = parse_row_number(digits)?;
        return Ok(A1Item::Cell(CellRef {
            sheet: None,
            row,
            col,
            row_abs,
            col_abs: first_abs,
        }));
    }
    let digits = cur.eat_while(|ch| ch.is_ascii_digit());
    if digits.is_empty() || !cur.is_empty() {
        return Err(CoreError::addr_parse(format!("invalid A1 item {s:?}")));
    }
    let row = parse_row_number(digits)?;
    Ok(A1Item::WholeRow {
        row,
        abs: first_abs,
    })
}

fn range_from_items(a: A1Item, b: A1Item) -> Result<RangeRef, CoreError> {
    match (a, b) {
        (A1Item::Cell(start), A1Item::Cell(end)) => Ok(RangeRef {
            start,
            end,
            sheet_end: None,
            whole_row: false,
            whole_col: false,
        }),
        (A1Item::WholeCol { col: c1, abs: a1 }, A1Item::WholeCol { col: c2, abs: a2 }) => {
            Ok(RangeRef {
                start: CellRef {
                    sheet: None,
                    row: 0,
                    col: c1,
                    row_abs: false,
                    col_abs: a1,
                },
                end: CellRef {
                    sheet: None,
                    row: MAX_ROWS - 1,
                    col: c2,
                    row_abs: false,
                    col_abs: a2,
                },
                sheet_end: None,
                whole_row: false,
                whole_col: true,
            })
        }
        (A1Item::WholeRow { row: r1, abs: a1 }, A1Item::WholeRow { row: r2, abs: a2 }) => {
            Ok(RangeRef {
                start: CellRef {
                    sheet: None,
                    row: r1,
                    col: 0,
                    row_abs: a1,
                    col_abs: false,
                },
                end: CellRef {
                    sheet: None,
                    row: r2,
                    col: MAX_COLS - 1,
                    row_abs: a2,
                    col_abs: false,
                },
                sheet_end: None,
                whole_row: true,
                whole_col: false,
            })
        }
        _ => Err(CoreError::addr_parse(
            "range sides must both be cells, both whole rows, or both whole columns",
        )),
    }
}

fn parse_row_number(digits: &str) -> Result<u32, CoreError> {
    let n: u32 = digits
        .parse()
        .map_err(|_| CoreError::addr_ref(format!("row {digits} is out of range")))?;
    if n == 0 || n > MAX_ROWS {
        Err(CoreError::addr_ref(format!("row {n} is out of range")))
    } else {
        Ok(n - 1)
    }
}

pub(super) fn split_sheet(input: &str) -> Result<(Option<SheetSpec>, &str), CoreError> {
    if let Some(stripped) = input.strip_prefix('\'') {
        let mut name = String::new();
        let mut i = 0usize;
        while i < stripped.len() {
            let Some(ch) = stripped[i..].chars().next() else {
                break;
            };
            if ch == '\'' {
                let next = stripped[i + 1..].chars().next();
                match next {
                    Some('\'') => {
                        name.push('\'');
                        i += 2;
                        continue;
                    }
                    Some('!') => {
                        let rest = &stripped[i + 2..];
                        if rest.is_empty() {
                            return Err(CoreError::addr_parse("missing address after sheet name"));
                        }
                        return Ok((Some(parse_sheet_spec(&name)?), rest));
                    }
                    _ => {
                        return Err(CoreError::addr_parse(
                            "quoted sheet name must be followed by '!'",
                        ));
                    }
                }
            }
            name.push(ch);
            i += ch.len_utf8();
        }
        return Err(CoreError::addr_parse("unterminated quoted sheet name"));
    }
    match input.find('!') {
        Some(0) => Err(CoreError::addr_parse("empty sheet name")),
        Some(bang) => {
            let name = &input[..bang];
            let rest = &input[bang + 1..];
            if rest.is_empty() {
                return Err(CoreError::addr_parse("missing address after sheet name"));
            }
            Ok((Some(parse_sheet_spec(name)?), rest))
        }
        None => Ok((None, input)),
    }
}

fn parse_sheet_spec(name: &str) -> Result<SheetSpec, CoreError> {
    if name.is_empty() {
        return Err(CoreError::addr_parse("empty sheet name"));
    }
    match name.split_once(':') {
        Some((start, end)) if !start.is_empty() && !end.is_empty() && !end.contains(':') => {
            Ok(SheetSpec {
                start: start.to_string(),
                end: Some(end.to_string()),
            })
        }
        Some(_) => Err(CoreError::addr_parse("invalid 3-D sheet span")),
        None => Ok(SheetSpec {
            start: name.to_string(),
            end: None,
        }),
    }
}

impl SheetSpec {
    /// `Sheet1!` or `'My Sheet'!` or `Sheet1:Sheet3!` prefix.
    #[must_use]
    pub fn to_a1_prefix(&self) -> String {
        match &self.end {
            None => format!("{}!", quote_sheet_name(&self.start)),
            Some(end) => {
                if sheet_name_needs_quote(&self.start) || sheet_name_needs_quote(end) {
                    format!(
                        "'{}:{}'!",
                        self.start.replace('\'', "''"),
                        end.replace('\'', "''")
                    )
                } else {
                    format!("{}:{}!", self.start, end)
                }
            }
        }
    }
}

/// Quote a sheet name when required for an unambiguous Excel reference.
///
/// Formula keywords such as `TRUE` and `FALSE` require quotes when used as
/// sheet names.
///
/// ```
/// use omacell_core::addr::quote_sheet_name;
/// assert_eq!(quote_sheet_name("Data"), "Data");
/// assert_eq!(quote_sheet_name("TRUE"), "'TRUE'");
/// ```
#[must_use]
pub fn quote_sheet_name(name: &str) -> String {
    if sheet_name_needs_quote(name) {
        format!("'{}'", name.replace('\'', "''"))
    } else {
        name.to_string()
    }
}

pub(super) fn format_col(col: u16, abs: bool) -> String {
    let letters = match col_to_letters(col) {
        Ok(s) => s,
        Err(_) => return "#REF!".to_string(),
    };
    if abs { format!("${letters}") } else { letters }
}

pub(super) fn format_row(row: u32, abs: bool) -> String {
    if row >= MAX_ROWS {
        return "#REF!".to_string();
    }
    let n = row + 1;
    if abs { format!("${n}") } else { n.to_string() }
}

impl CellRef {
    /// A1 text without a sheet prefix (`$A$1`).
    #[must_use]
    pub fn to_a1(self) -> String {
        if self.validate().is_err() {
            return "#REF!".to_string();
        }
        let mut s = format_col(self.col, self.col_abs);
        if s == "#REF!" {
            return s;
        }
        if self.row_abs {
            s.push('$');
        }
        s.push_str(&(self.row + 1).to_string());
        s
    }
}

impl RangeRef {
    /// A1 text without a sheet prefix (`A1:B2`, `A:A`, `1:1`).
    #[must_use]
    pub fn to_a1(self) -> String {
        if self.whole_col && !self.whole_row {
            format!(
                "{}:{}",
                format_col(self.start.col, self.start.col_abs),
                format_col(self.end.col, self.end.col_abs)
            )
        } else if self.whole_row && !self.whole_col {
            format!(
                "{}:{}",
                format_row(self.start.row, self.start.row_abs),
                format_row(self.end.row, self.end.row_abs)
            )
        } else {
            format!("{}:{}", self.start.to_a1(), self.end.to_a1())
        }
    }
}

impl ParsedRef {
    /// Canonical A1 text, including a sheet prefix when present.
    #[must_use]
    pub fn to_a1(&self) -> String {
        let body = match self.kind {
            RefKind::Cell(c) => c.to_a1(),
            RefKind::Range(r) => r.to_a1(),
        };
        match &self.sheet {
            Some(sheet) => format!("{}{}", sheet.to_a1_prefix(), body),
            None => body,
        }
    }
}

impl std::fmt::Display for CellRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_a1())
    }
}

impl std::fmt::Display for RangeRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_a1())
    }
}
