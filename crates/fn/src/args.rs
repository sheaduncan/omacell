//! Shared argument/array helpers for WP-05c. Named `args` so parallel WP-05a
//! (`common`) and WP-05b (`util`) helpers do not collide.

use std::sync::Arc;

use omacell_core::addr::{RefKind, SheetId, parse_a1, parse_r1c1};
use omacell_core::coerce::{self, Cmp, Scalar};
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, FnRegistry, Reference, RuntimeArray, RuntimeValue};
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use omacell_core::workbook::Workbook;

use crate::FunctionSpec;

/// Register a spec and its aliases.
pub fn register_spec(registry: &mut FnRegistry, spec: &FunctionSpec) {
    registry.register(spec.to_fn_def());
    for alias in spec.aliases {
        let mut def = spec.to_fn_def();
        def.name = alias;
        registry.register(def);
    }
}

/// Whether the slot is missing or marked omitted.
pub fn is_omitted(args: &[ArgVal], index: usize) -> bool {
    match args.get(index) {
        None => true,
        Some(a) => a.omitted,
    }
}

/// Coerce an argument to a single scalar (1×1 arrays unwrap). Errors propagate.
pub fn scalar(ctx: &mut EvalCtx<'_>, arg: &ArgVal) -> Result<Scalar, ErrorKind> {
    if arg.omitted {
        return Ok(Scalar::Empty);
    }
    match ctx.materialize(arg.value.clone()) {
        RuntimeValue::Scalar(s) => match s.error() {
            Some(e) => Err(e),
            None => Ok(s),
        },
        RuntimeValue::Array(array) => {
            array.validate()?;
            if array.rows == 1 && array.cols == 1 {
                let s = array.values.first().cloned().unwrap_or(Scalar::Empty);
                match s.error() {
                    Some(e) => Err(e),
                    None => Ok(s),
                }
            } else {
                Err(ErrorKind::Value)
            }
        }
        RuntimeValue::Lambda(_) => Err(ErrorKind::Value),
        RuntimeValue::Ref(_) => Err(ErrorKind::Value),
    }
}

/// Optional scalar; omitted/missing yields `None` (not empty).
pub fn opt_scalar(
    ctx: &mut EvalCtx<'_>,
    args: &[ArgVal],
    index: usize,
) -> Result<Option<Scalar>, ErrorKind> {
    match args.get(index) {
        None | Some(ArgVal { omitted: true, .. }) => Ok(None),
        Some(arg) => scalar(ctx, arg).map(Some),
    }
}

/// Coerce an argument to a finite number (empty → 0).
pub fn number(ctx: &mut EvalCtx<'_>, arg: &ArgVal) -> Result<f64, ErrorKind> {
    coerce::to_number(&scalar(ctx, arg)?)
}

/// Optional number with a default when omitted.
pub fn opt_number(
    ctx: &mut EvalCtx<'_>,
    args: &[ArgVal],
    index: usize,
    default: f64,
) -> Result<f64, ErrorKind> {
    match opt_scalar(ctx, args, index)? {
        None => Ok(default),
        Some(s) => coerce::to_number(&s),
    }
}

/// Optional bool with a default when omitted.
pub fn opt_bool(
    ctx: &mut EvalCtx<'_>,
    args: &[ArgVal],
    index: usize,
    default: bool,
) -> Result<bool, ErrorKind> {
    match opt_scalar(ctx, args, index)? {
        None => Ok(default),
        Some(s) => coerce::to_bool(&s),
    }
}

/// Truncate toward zero; require a finite number.
pub fn trunc_i64(n: f64) -> Result<i64, ErrorKind> {
    if !n.is_finite() {
        return Err(ErrorKind::Num);
    }
    Ok(n.trunc() as i64)
}

/// Require a positive (`>= 1`) truncated dimension that fits `u32`.
pub fn pos_u32(n: f64) -> Result<u32, ErrorKind> {
    let v = trunc_i64(n)?;
    if v < 1 {
        return Err(ErrorKind::Num);
    }
    u32::try_from(v).map_err(|_| ErrorKind::Num)
}

/// Build a runtime array after checking the shape (no allocation on failure).
pub fn array_result(rows: u32, cols: u32, values: Vec<Scalar>) -> RuntimeValue {
    RuntimeValue::array(rows, cols, values)
}

/// Reject an invalid output shape **before** allocating the payload.
pub fn check_shape(rows: u32, cols: u32) -> Result<usize, ErrorKind> {
    RuntimeArray::checked_len(rows, cols)
}

/// Convert a runtime value into a validated array (scalars become 1×1).
pub fn to_array(ctx: &mut EvalCtx<'_>, value: RuntimeValue) -> Result<RuntimeArray, ErrorKind> {
    match ctx.materialize(value) {
        RuntimeValue::Scalar(s) => {
            if let Some(e) = s.error() {
                return Err(e);
            }
            RuntimeArray::try_new(1, 1, vec![s])
        }
        RuntimeValue::Array(a) => {
            a.validate()?;
            Ok((*a).clone())
        }
        RuntimeValue::Lambda(_) => Err(ErrorKind::Value),
        RuntimeValue::Ref(_) => Err(ErrorKind::Value),
    }
}

/// Array from an argument (omitted → 1×1 empty).
pub fn arg_array(ctx: &mut EvalCtx<'_>, arg: &ArgVal) -> Result<RuntimeArray, ErrorKind> {
    if arg.omitted {
        return RuntimeArray::try_new(1, 1, vec![Scalar::Empty]);
    }
    if let Some(e) = arg.value.error_kind() {
        return Err(e);
    }
    to_array(ctx, arg.value.clone())
}

/// Inclusive rectangle dimensions.
pub fn rect_shape(r1: u32, r2: u32, c1: u16, c2: u16) -> (u32, u32) {
    let rows = r1.abs_diff(r2) + 1;
    let cols = u32::from(c1.abs_diff(c2)) + 1;
    (rows, cols)
}

/// Shape of a reference without materializing cells.
pub fn reference_shape(r: &Reference) -> Result<(u32, u32), ErrorKind> {
    match r {
        Reference::Range {
            start_row,
            end_row,
            start_col,
            end_col,
            ..
        } => Ok(rect_shape(*start_row, *end_row, *start_col, *end_col)),
        Reference::Union(parts) if parts.len() == 1 => reference_shape(&parts[0]),
        Reference::Union(_) => Err(ErrorKind::Value),
        Reference::ThreeD {
            sheets,
            start_row,
            end_row,
            start_col,
            end_col,
        } => {
            let (h, w) = rect_shape(*start_row, *end_row, *start_col, *end_col);
            let n = u32::try_from(sheets.len()).map_err(|_| ErrorKind::Num)?;
            let rows = h.checked_mul(n).ok_or(ErrorKind::Num)?;
            Ok((rows, w))
        }
    }
}

/// Count union areas (Excel `AREAS`).
pub fn area_count(r: &Reference) -> u32 {
    match r {
        Reference::Union(parts) => parts.iter().map(area_count).sum::<u32>().max(1),
        _ => 1,
    }
}

/// Nth area (1-based) of a union; `INDEX`'s `area_num`.
pub fn nth_area(r: &Reference, area_num: i64) -> Result<&Reference, ErrorKind> {
    if area_num < 1 {
        return Err(ErrorKind::Ref);
    }
    match r {
        Reference::Union(parts) => {
            let idx = usize::try_from(area_num - 1).map_err(|_| ErrorKind::Ref)?;
            parts.get(idx).ok_or(ErrorKind::Ref)
        }
        other if area_num == 1 => Ok(other),
        _ => Err(ErrorKind::Ref),
    }
}

/// Require a vector (single row or single column).
pub fn as_vector(array: &RuntimeArray) -> Result<Vec<Scalar>, ErrorKind> {
    if array.rows == 1 || array.cols == 1 {
        Ok(array.values.iter().cloned().collect())
    } else {
        Err(ErrorKind::Value)
    }
}

/// Cell at 0-based (row, col) in a runtime array.
pub fn at(array: &RuntimeArray, row: u32, col: u32) -> Scalar {
    let idx = (row as usize)
        .saturating_mul(array.cols as usize)
        .saturating_add(col as usize);
    array.values.get(idx).cloned().unwrap_or(Scalar::Empty)
}

/// Type-strict exact match (case-insensitive text). `"1"` does not equal `1`.
pub fn exact_eq(a: &Scalar, b: &Scalar) -> bool {
    match (a, b) {
        (Scalar::Error(x), Scalar::Error(y)) => x == y,
        (Scalar::Error(_), _) | (_, Scalar::Error(_)) => false,
        (Scalar::Number(x), Scalar::Number(y)) => x == y,
        (Scalar::Bool(x), Scalar::Bool(y)) => x == y,
        (Scalar::Text(x), Scalar::Text(y)) => x
            .chars()
            .flat_map(char::to_lowercase)
            .eq(y.chars().flat_map(char::to_lowercase)),
        (Scalar::Empty, Scalar::Empty) => true,
        (Scalar::Empty, Scalar::Text(t)) | (Scalar::Text(t), Scalar::Empty) => t.is_empty(),
        _ => false,
    }
}

/// Excel wildcard match (`*`, `?`, `~`), case-insensitive.
pub fn wildcard_eq(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let t: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    wildcard(&p, &t)
}

fn wildcard(pat: &[char], text: &[char]) -> bool {
    let mut i = 0;
    let mut j = 0;
    let mut star = None;
    while j < text.len() {
        if i < pat.len() && pat[i] == '~' && i + 1 < pat.len() {
            if pat[i + 1] == text[j] {
                i += 2;
                j += 1;
                continue;
            }
            if let Some((si, sj)) = star {
                i = si;
                j = sj + 1;
                star = Some((si, j));
                continue;
            }
            return false;
        }
        if i < pat.len() && (pat[i] == '?' || pat[i] == text[j]) {
            i += 1;
            j += 1;
            continue;
        }
        if i < pat.len() && pat[i] == '*' {
            i += 1;
            star = Some((i, j));
            continue;
        }
        if let Some((si, sj)) = star {
            i = si;
            j = sj + 1;
            star = Some((si, j));
            continue;
        }
        return false;
    }
    while i < pat.len() && pat[i] == '*' {
        i += 1;
    }
    i == pat.len()
}

/// Coerce a scalar to text for wildcard matching.
pub fn as_text(s: &Scalar) -> Result<Arc<str>, ErrorKind> {
    coerce::to_text(s)
}

/// Last index whose key is `<=` lookup using binary search (Excel approximate).
pub fn last_le(keys: &[Scalar], lookup: &Scalar) -> Result<Option<usize>, ErrorKind> {
    if keys.is_empty() {
        return Ok(None);
    }
    let mut lo = 0usize;
    let mut hi = keys.len();
    let mut found = None;
    while lo < hi {
        let mid = (lo + hi) / 2;
        match coerce::compare(&keys[mid], lookup) {
            Err(e) => return Err(e),
            Ok(Cmp::Lt | Cmp::Eq) => {
                found = Some(mid);
                lo = mid + 1;
            }
            Ok(Cmp::Gt) => hi = mid,
        }
    }
    Ok(found)
}

/// Last original index whose non-error key is `<=` lookup.
pub fn last_le_skipping_errors(
    keys: &[Scalar],
    lookup: &Scalar,
) -> Result<Option<usize>, ErrorKind> {
    let comparable: Vec<(usize, &Scalar)> = keys
        .iter()
        .enumerate()
        .filter(|(_, key)| !matches!(key, Scalar::Error(_)))
        .collect();
    let mut lo = 0usize;
    let mut hi = comparable.len();
    let mut found = None;
    while lo < hi {
        let mid = (lo + hi) / 2;
        match coerce::compare(comparable[mid].1, lookup)? {
            Cmp::Lt | Cmp::Eq => {
                found = Some(comparable[mid].0);
                lo = mid + 1;
            }
            Cmp::Gt => hi = mid,
        }
    }
    Ok(found)
}

/// Last index whose key is `>=` lookup, for descending-sorted MATCH type `-1`.
pub fn last_ge_desc(keys: &[Scalar], lookup: &Scalar) -> Result<Option<usize>, ErrorKind> {
    if keys.is_empty() {
        return Ok(None);
    }
    let mut lo = 0usize;
    let mut hi = keys.len();
    let mut found = None;
    while lo < hi {
        let mid = (lo + hi) / 2;
        match coerce::compare(&keys[mid], lookup) {
            Err(e) => return Err(e),
            Ok(Cmp::Gt | Cmp::Eq) => {
                found = Some(mid);
                lo = mid + 1;
            }
            Ok(Cmp::Lt) => hi = mid,
        }
    }
    Ok(found)
}

/// Whether a scalar is blank for TOCOL/TOROW ignore=1.
pub fn is_blank(s: &Scalar) -> bool {
    match s {
        Scalar::Empty => true,
        Scalar::Text(t) if t.is_empty() => true,
        _ => false,
    }
}

/// Single-cell reference.
pub fn cell_ref(sheet: SheetId, row: u32, col: u16) -> Reference {
    Reference::Range {
        sheet,
        start_row: row,
        start_col: col,
        end_row: row,
        end_col: col,
    }
}

/// Resolve an A1 or R1C1 address text into a reference on `sheet`.
pub fn parse_address(
    wb: &Workbook,
    text: &str,
    a1: bool,
    base_row: u32,
    base_col: u16,
    default_sheet: SheetId,
) -> Result<Reference, ErrorKind> {
    let parsed = if a1 {
        parse_a1(text).map_err(|_| ErrorKind::Ref)?
    } else {
        parse_r1c1(text, base_row, base_col).map_err(|_| ErrorKind::Ref)?
    };
    let sheet = match &parsed.sheet {
        Some(spec) if spec.end.is_some() => {
            let start = wb
                .resolve_sheet_name(&spec.start)
                .map_err(|_| ErrorKind::Ref)?;
            let end_name = spec.end.as_deref().unwrap_or(spec.start.as_str());
            let end = wb
                .resolve_sheet_name(end_name)
                .map_err(|_| ErrorKind::Ref)?;
            let ids: Vec<SheetId> = wb.sheets().map(|s| s.id).collect();
            let i = ids.iter().position(|&x| x == start).ok_or(ErrorKind::Ref)?;
            let j = ids.iter().position(|&x| x == end).ok_or(ErrorKind::Ref)?;
            let (a, b) = if i <= j { (i, j) } else { (j, i) };
            let sheets = ids[a..=b].to_vec();
            return match parsed.kind {
                RefKind::Cell(c) => Ok(Reference::ThreeD {
                    sheets,
                    start_row: c.row,
                    start_col: c.col,
                    end_row: c.row,
                    end_col: c.col,
                }),
                RefKind::Range(r) => Ok(Reference::ThreeD {
                    sheets,
                    start_row: r.start.row,
                    start_col: r.start.col,
                    end_row: r.end.row,
                    end_col: r.end.col,
                }),
            };
        }
        Some(spec) => wb
            .resolve_sheet_name(&spec.start)
            .map_err(|_| ErrorKind::Ref)?,
        None => default_sheet,
    };
    Ok(match parsed.kind {
        RefKind::Cell(c) => cell_ref(sheet, c.row, c.col),
        RefKind::Range(r) => Reference::Range {
            sheet,
            start_row: r.start.row,
            start_col: r.start.col,
            end_row: r.end.row,
            end_col: r.end.col,
        },
    })
}

/// Offset a range; negative size extends in the opposite direction.
pub fn offset_ref(
    r: &Reference,
    row_off: i64,
    col_off: i64,
    height: Option<i64>,
    width: Option<i64>,
) -> Result<Reference, ErrorKind> {
    let Reference::Range {
        sheet,
        start_row,
        start_col,
        end_row,
        end_col,
    } = r
    else {
        return match r {
            Reference::Union(parts) if parts.len() == 1 => {
                offset_ref(&parts[0], row_off, col_off, height, width)
            }
            _ => Err(ErrorKind::Value),
        };
    };
    let r1 = (*start_row).min(*end_row) as i64;
    let r2 = (*start_row).max(*end_row) as i64;
    let c1 = i64::from((*start_col).min(*end_col));
    let c2 = i64::from((*start_col).max(*end_col));
    let orig_h = r2 - r1 + 1;
    let orig_w = c2 - c1 + 1;
    let nr1 = r1.checked_add(row_off).ok_or(ErrorKind::Ref)?;
    let nc1 = c1.checked_add(col_off).ok_or(ErrorKind::Ref)?;
    let h = height.unwrap_or(orig_h);
    let w = width.unwrap_or(orig_w);
    let (nr1, nr2) = span(nr1, h)?;
    let (nc1, nc2) = span(nc1, w)?;
    if nr1 < 0 || nr2 >= i64::from(MAX_ROWS) || nc1 < 0 || nc2 >= i64::from(MAX_COLS) {
        return Err(ErrorKind::Ref);
    }
    Ok(Reference::Range {
        sheet: *sheet,
        start_row: nr1 as u32,
        start_col: nc1 as u16,
        end_row: nr2 as u32,
        end_col: nc2 as u16,
    })
}

fn span(start: i64, size: i64) -> Result<(i64, i64), ErrorKind> {
    if size == 0 {
        return Err(ErrorKind::Ref);
    }
    if size > 0 {
        let end = start.checked_add(size - 1).ok_or(ErrorKind::Ref)?;
        Ok((start, end))
    } else {
        let end = start;
        let start = start.checked_add(size + 1).ok_or(ErrorKind::Ref)?;
        Ok((start, end))
    }
}

/// Empty-array result used by TAKE/DROP/FILTER/UNIQUE (`#CALC!`).
pub fn empty_array() -> RuntimeValue {
    RuntimeValue::error(ErrorKind::Calc)
}

/// Take a lambda from an argument.
pub fn lambda_of(arg: &ArgVal) -> Result<Arc<omacell_core::lambda::Lambda>, ErrorKind> {
    if arg.omitted {
        return Err(ErrorKind::Value);
    }
    match &arg.value {
        RuntimeValue::Lambda(l) => Ok(Arc::clone(l)),
        RuntimeValue::Scalar(Scalar::Error(e)) => Err(*e),
        _ => Err(ErrorKind::Value),
    }
}

/// Apply a lambda to already-evaluated arguments.
pub fn apply_lambda(
    ctx: &mut EvalCtx<'_>,
    lam: &omacell_core::lambda::Lambda,
    values: Vec<RuntimeValue>,
) -> RuntimeValue {
    let args: Vec<ArgVal> = values
        .into_iter()
        .map(|value| ArgVal {
            omitted: false,
            value,
        })
        .collect();
    omacell_core::lambda::apply(ctx, lam, &args)
}
