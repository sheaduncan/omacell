//! R1C1 parse and print with relative offsets resolved against a base cell.

use crate::error::CoreError;
use crate::limits::{MAX_COLS, MAX_ROWS};

use super::a1::split_sheet;
use super::scan::Cursor;
use super::{CellRef, ParsedRef, RangeRef, RefKind};

/// Parse an R1C1 cell or range relative to `base_row` / `base_col` (0-based).
///
/// Absolute tokens (`R1C1`) ignore the base. Relative tokens (`R[-1]C[2]`)
/// are resolved into grid indices; landing outside the grid is `#REF!`.
///
/// ```
/// use omacell_core::addr::{parse_r1c1, RefKind};
/// let parsed = parse_r1c1("RC[1]", 0, 0).unwrap();
/// match parsed.kind {
///     RefKind::Cell(c) => assert_eq!((c.row, c.col), (0, 1)),
///     _ => panic!("cell"),
/// }
/// ```
pub fn parse_r1c1(input: &str, base_row: u32, base_col: u16) -> Result<ParsedRef, CoreError> {
    if base_row >= MAX_ROWS || u32::from(base_col) >= u32::from(MAX_COLS) {
        return Err(CoreError::addr_ref(format!(
            "R1C1 base r{base_row}c{base_col} is out of range"
        )));
    }
    let input = input.trim();
    if input.is_empty() {
        return Err(CoreError::addr_parse("empty address"));
    }
    let (sheet, body) = split_sheet(input)?;
    let kind = parse_r1c1_body(body, base_row, base_col)?;
    Ok(ParsedRef { sheet, kind })
}

/// Parse a single R1C1 cell. Whole-row/column forms are rejected.
///
/// ```
/// use omacell_core::addr::parse_r1c1_cell;
/// let cell = parse_r1c1_cell("R1C1", 0, 0).unwrap();
/// assert_eq!((cell.row, cell.col), (0, 0));
/// ```
pub fn parse_r1c1_cell(input: &str, base_row: u32, base_col: u16) -> Result<CellRef, CoreError> {
    let parsed = parse_r1c1(input, base_row, base_col)?;
    if parsed.sheet.is_some() {
        return Err(CoreError::addr_parse(
            "cell-only parser does not accept a sheet qualifier; use parse_r1c1",
        ));
    }
    match parsed.kind {
        RefKind::Cell(cell) => Ok(cell),
        RefKind::Range(_) => Err(CoreError::addr_parse(
            "expected a cell address, got a range",
        )),
    }
}

pub(super) fn parse_r1c1_body(
    body: &str,
    base_row: u32,
    base_col: u16,
) -> Result<RefKind, CoreError> {
    if body.is_empty() {
        return Err(CoreError::addr_parse("empty address body"));
    }
    match body.split_once(':') {
        Some((left, right)) => {
            let a = parse_r1c1_item(left, base_row, base_col)?;
            let b = parse_r1c1_item(right, base_row, base_col)?;
            Ok(RefKind::Range(range_from_items(a, b)?))
        }
        None => match parse_r1c1_item(body, base_row, base_col)? {
            R1Item::Cell(cell) => Ok(RefKind::Cell(cell)),
            other => Ok(RefKind::Range(range_from_items(other, other)?)),
        },
    }
}

#[derive(Clone, Copy)]
enum R1Item {
    Cell(CellRef),
    WholeCol { col: u16, abs: bool },
    WholeRow { row: u32, abs: bool },
}

fn parse_r1c1_item(s: &str, base_row: u32, base_col: u16) -> Result<R1Item, CoreError> {
    let mut cur = Cursor::new(s);
    let row = if cur.eat_char_ci('R') {
        Some(parse_axis(&mut cur, base_row, MAX_ROWS)?)
    } else {
        None
    };
    let col = if cur.eat_char_ci('C') {
        let (idx, abs) = parse_axis(&mut cur, u32::from(base_col), u32::from(MAX_COLS))?;
        let col = u16::try_from(idx)
            .map_err(|_| CoreError::addr_ref(format!("column {idx} is out of range")))?;
        Some((col, abs))
    } else {
        None
    };
    if !cur.is_empty() {
        return Err(CoreError::addr_parse(format!("invalid R1C1 item {s:?}")));
    }
    match (row, col) {
        (Some((row, row_abs)), Some((col, col_abs))) => Ok(R1Item::Cell(CellRef {
            sheet: None,
            row,
            col,
            row_abs,
            col_abs,
        })),
        (Some((row, abs)), None) => Ok(R1Item::WholeRow { row, abs }),
        (None, Some((col, abs))) => Ok(R1Item::WholeCol { col, abs }),
        (None, None) => Err(CoreError::addr_parse(format!("invalid R1C1 item {s:?}"))),
    }
}

fn parse_axis(cur: &mut Cursor<'_>, base: u32, count: u32) -> Result<(u32, bool), CoreError> {
    if cur.eat_char('[') {
        let num = cur.eat_while(|ch| ch == '-' || ch.is_ascii_digit());
        if !cur.eat_char(']') {
            return Err(CoreError::addr_parse("unterminated R1C1 offset"));
        }
        if !is_signed_int(num) {
            return Err(CoreError::addr_parse(format!(
                "invalid R1C1 offset {num:?}"
            )));
        }
        let off: i32 = num
            .parse()
            .map_err(|_| CoreError::addr_parse(format!("invalid R1C1 offset {num:?}")))?;
        Ok((add_offset(base, off, count)?, false))
    } else {
        let digits = cur.eat_while(|ch| ch.is_ascii_digit());
        if digits.is_empty() {
            Ok((base, false))
        } else {
            let n: u32 = digits
                .parse()
                .map_err(|_| CoreError::addr_ref(format!("index {digits} is out of range")))?;
            if n == 0 || n > count {
                Err(CoreError::addr_ref(format!("index {n} is out of range")))
            } else {
                Ok((n - 1, true))
            }
        }
    }
}

fn is_signed_int(s: &str) -> bool {
    if s.is_empty() || s == "-" {
        return false;
    }
    let rest = s.strip_prefix('-').unwrap_or(s);
    !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit())
}

fn add_offset(base: u32, off: i32, count: u32) -> Result<u32, CoreError> {
    let v = i64::from(base) + i64::from(off);
    if v < 0 || v >= i64::from(count) {
        Err(CoreError::addr_ref(format!(
            "relative reference {base}{off:+} is out of range"
        )))
    } else {
        Ok(v as u32)
    }
}

fn range_from_items(a: R1Item, b: R1Item) -> Result<RangeRef, CoreError> {
    match (a, b) {
        (R1Item::Cell(start), R1Item::Cell(end)) => Ok(RangeRef {
            start,
            end,
            sheet_end: None,
            whole_row: false,
            whole_col: false,
        }),
        (R1Item::WholeCol { col: c1, abs: a1 }, R1Item::WholeCol { col: c2, abs: a2 }) => {
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
        (R1Item::WholeRow { row: r1, abs: a1 }, R1Item::WholeRow { row: r2, abs: a2 }) => {
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

fn print_axis(letter: char, index: u32, abs: bool, base: u32) -> String {
    if abs {
        format!("{letter}{}", index + 1)
    } else {
        let off = i64::from(index) - i64::from(base);
        if off == 0 {
            letter.to_string()
        } else {
            format!("{letter}[{off}]")
        }
    }
}

impl CellRef {
    /// R1C1 text without a sheet prefix, relative to `base_row` / `base_col`.
    #[must_use]
    pub fn to_r1c1(self, base_row: u32, base_col: u16) -> String {
        if self.validate().is_err()
            || base_row >= MAX_ROWS
            || u32::from(base_col) >= u32::from(MAX_COLS)
        {
            return "#REF!".to_string();
        }
        format!(
            "{}{}",
            print_axis('R', self.row, self.row_abs, base_row),
            print_axis('C', u32::from(self.col), self.col_abs, u32::from(base_col))
        )
    }
}

impl RangeRef {
    /// R1C1 text without a sheet prefix.
    #[must_use]
    pub fn to_r1c1(self, base_row: u32, base_col: u16) -> String {
        if self.whole_col && !self.whole_row {
            let a = print_axis(
                'C',
                u32::from(self.start.col),
                self.start.col_abs,
                u32::from(base_col),
            );
            let b = print_axis(
                'C',
                u32::from(self.end.col),
                self.end.col_abs,
                u32::from(base_col),
            );
            if a == b { a } else { format!("{a}:{b}") }
        } else if self.whole_row && !self.whole_col {
            let a = print_axis('R', self.start.row, self.start.row_abs, base_row);
            let b = print_axis('R', self.end.row, self.end.row_abs, base_row);
            if a == b { a } else { format!("{a}:{b}") }
        } else {
            format!(
                "{}:{}",
                self.start.to_r1c1(base_row, base_col),
                self.end.to_r1c1(base_row, base_col)
            )
        }
    }
}

impl ParsedRef {
    /// Canonical R1C1 text, including a sheet prefix when present.
    #[must_use]
    pub fn to_r1c1(&self, base_row: u32, base_col: u16) -> String {
        let body = match self.kind {
            RefKind::Cell(c) => c.to_r1c1(base_row, base_col),
            RefKind::Range(r) => r.to_r1c1(base_row, base_col),
        };
        match &self.sheet {
            Some(sheet) => format!("{}{}", sheet.to_a1_prefix(), body),
            None => body,
        }
    }
}
