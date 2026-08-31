//! Lookup and reference functions (WP-05c).

use omacell_core::addr::{col_to_letters, quote_sheet_name};
use omacell_core::coerce::{self, Scalar};
use omacell_core::error::ErrorKind;
use omacell_core::eval::{
    ArgVal, EvalCtx, FnBody, FnRegistry, Reference, RuntimeArray, RuntimeValue,
};
use omacell_core::limits::{MAX_COLS, MAX_ROWS};

use crate::args;
use crate::metadata::{ArgKind, ArrayBehavior, FunctionSpec};

/// Lookup/reference specs in declaration order.
pub const SPECS: &[FunctionSpec] = &[
    XLOOKUP, XMATCH, INDEX, MATCH, VLOOKUP, HLOOKUP, LOOKUP, CHOOSE, OFFSET, INDIRECT, ROW, ROWS,
    COLUMN, COLUMNS, ADDRESS, AREAS,
];

/// Register lookup/reference functions.
pub fn register_lookup(registry: &mut FnRegistry) {
    for spec in SPECS {
        args::register_spec(registry, spec);
    }
}

crate::define_fn! {
const XLOOKUP = {
    name: "XLOOKUP",
    aliases: &[],
    tier: 0,
    category: "lookup",
    arg_kinds: &[ArgKind::Any, ArgKind::Array, ArgKind::Array, ArgKind::Any, ArgKind::Number, ArgKind::Number],
    min_args: 3,
    max_args: 6,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "XLOOKUP(lookup_value, lookup_array, return_array, [if_not_found], [match_mode], [search_mode])",
    doc: "Looks up a value and returns a corresponding result from another array.",
    body: FnBody::Eager(xlookup_impl),
};
}

crate::define_fn! {
const XMATCH = {
    name: "XMATCH",
    aliases: &[],
    tier: 0,
    category: "lookup",
    arg_kinds: &[ArgKind::Any, ArgKind::Array, ArgKind::Number, ArgKind::Number],
    min_args: 2,
    max_args: 4,
    volatile: false,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "XMATCH(lookup_value, lookup_array, [match_mode], [search_mode])",
    doc: "Returns the relative position of a lookup value in an array.",
    body: FnBody::Eager(xmatch_impl),
};
}

crate::define_fn! {
const INDEX = {
    name: "INDEX",
    aliases: &[],
    tier: 0,
    category: "lookup",
    arg_kinds: &[ArgKind::Any, ArgKind::Number, ArgKind::Number, ArgKind::Number],
    min_args: 1,
    max_args: 4,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "INDEX(array, [row_num], [column_num], [area_num])",
    doc: "Returns a value or reference from a table or array by row and column.",
    body: FnBody::Eager(index_impl),
};
}

crate::define_fn! {
const MATCH = {
    name: "MATCH",
    aliases: &[],
    tier: 0,
    category: "lookup",
    arg_kinds: &[ArgKind::Any, ArgKind::Array, ArgKind::Number],
    min_args: 2,
    max_args: 3,
    volatile: false,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "MATCH(lookup_value, lookup_array, [match_type])",
    doc: "Returns the relative position of a lookup value in a vector.",
    body: FnBody::Eager(match_impl),
};
}

crate::define_fn! {
const VLOOKUP = {
    name: "VLOOKUP",
    aliases: &[],
    tier: 0,
    category: "lookup",
    arg_kinds: &[ArgKind::Any, ArgKind::Array, ArgKind::Number, ArgKind::Logical],
    min_args: 3,
    max_args: 4,
    volatile: false,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "VLOOKUP(lookup_value, table_array, col_index_num, [range_lookup])",
    doc: "Looks up a value in the first column of a table and returns a value in the same row.",
    body: FnBody::Eager(vlookup_impl),
};
}

crate::define_fn! {
const HLOOKUP = {
    name: "HLOOKUP",
    aliases: &[],
    tier: 0,
    category: "lookup",
    arg_kinds: &[ArgKind::Any, ArgKind::Array, ArgKind::Number, ArgKind::Logical],
    min_args: 3,
    max_args: 4,
    volatile: false,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "HLOOKUP(lookup_value, table_array, row_index_num, [range_lookup])",
    doc: "Looks up a value in the first row of a table and returns a value in the same column.",
    body: FnBody::Eager(hlookup_impl),
};
}

crate::define_fn! {
const LOOKUP = {
    name: "LOOKUP",
    aliases: &[],
    tier: 0,
    category: "lookup",
    arg_kinds: &[ArgKind::Any, ArgKind::Array, ArgKind::Array],
    min_args: 2,
    max_args: 3,
    volatile: false,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "LOOKUP(lookup_value, lookup_vector, [result_vector])",
    doc: "Approximate lookup in a sorted vector (vector form) or array (array form).",
    body: FnBody::Eager(lookup_impl),
};
}

crate::define_fn! {
const CHOOSE = {
    name: "CHOOSE",
    aliases: &[],
    tier: 0,
    category: "lookup",
    arg_kinds: &[ArgKind::Number, ArgKind::Any],
    min_args: 2,
    max_args: 255,
    volatile: false,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "CHOOSE(index_num, value1, [value2], ...)",
    doc: "Returns the value at the given 1-based index.",
    body: FnBody::Eager(choose_impl),
};
}

crate::define_fn! {
const OFFSET = {
    name: "OFFSET",
    aliases: &[],
    tier: 0,
    category: "lookup",
    arg_kinds: &[ArgKind::Range, ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number],
    min_args: 3,
    max_args: 5,
    volatile: true,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "OFFSET(reference, rows, cols, [height], [width])",
    doc: "Returns a reference offset from a starting reference. Volatile.",
    body: FnBody::Eager(offset_impl),
};
}

crate::define_fn! {
const INDIRECT = {
    name: "INDIRECT",
    aliases: &[],
    tier: 0,
    category: "lookup",
    arg_kinds: &[ArgKind::Text, ArgKind::Logical],
    min_args: 1,
    max_args: 2,
    volatile: true,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "INDIRECT(ref_text, [a1])",
    doc: "Returns the reference specified by a text string. Volatile.",
    body: FnBody::Eager(indirect_impl),
};
}

crate::define_fn! {
const ROW = {
    name: "ROW",
    aliases: &[],
    tier: 0,
    category: "lookup",
    arg_kinds: &[ArgKind::Range],
    min_args: 0,
    max_args: 1,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "ROW([reference])",
    doc: "Returns the row number of a reference, or of the formula cell if omitted.",
    body: FnBody::Eager(row_impl),
};
}

crate::define_fn! {
const ROWS = {
    name: "ROWS",
    aliases: &[],
    tier: 0,
    category: "lookup",
    arg_kinds: &[ArgKind::Any],
    min_args: 1,
    max_args: 1,
    volatile: false,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "ROWS(array)",
    doc: "Returns the number of rows in a reference or array without materializing values.",
    body: FnBody::Eager(rows_impl),
};
}

crate::define_fn! {
const COLUMN = {
    name: "COLUMN",
    aliases: &[],
    tier: 0,
    category: "lookup",
    arg_kinds: &[ArgKind::Range],
    min_args: 0,
    max_args: 1,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "COLUMN([reference])",
    doc: "Returns the column number of a reference, or of the formula cell if omitted.",
    body: FnBody::Eager(column_impl),
};
}

crate::define_fn! {
const COLUMNS = {
    name: "COLUMNS",
    aliases: &[],
    tier: 0,
    category: "lookup",
    arg_kinds: &[ArgKind::Any],
    min_args: 1,
    max_args: 1,
    volatile: false,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "COLUMNS(array)",
    doc: "Returns the number of columns in a reference or array without materializing values.",
    body: FnBody::Eager(columns_impl),
};
}

crate::define_fn! {
const ADDRESS = {
    name: "ADDRESS",
    aliases: &[],
    tier: 0,
    category: "lookup",
    arg_kinds: &[ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Logical, ArgKind::Text],
    min_args: 2,
    max_args: 5,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "ADDRESS(row_num, column_num, [abs_num], [a1], [sheet_text])",
    doc: "Builds a cell address as text.",
    body: FnBody::Eager(address_impl),
};
}

crate::define_fn! {
const AREAS = {
    name: "AREAS",
    aliases: &[],
    tier: 0,
    category: "lookup",
    arg_kinds: &[ArgKind::Range],
    min_args: 1,
    max_args: 1,
    volatile: false,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "AREAS(reference)",
    doc: "Returns the number of areas in a reference.",
    body: FnBody::Eager(areas_impl),
};
}

fn err(e: ErrorKind) -> RuntimeValue {
    RuntimeValue::error(e)
}

fn xlookup_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    if let Some(e) = args.first().and_then(|a| a.value.error_kind()) {
        return err(e);
    }
    let lookup = match args.first() {
        Some(a) => match args::scalar(ctx, a) {
            Ok(s) => s,
            Err(e) => return err(e),
        },
        None => return err(ErrorKind::Value),
    };
    let lookup_arr = match args
        .get(1)
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::arg_array(ctx, a))
    {
        Ok(a) => a,
        Err(e) => return err(e),
    };
    let return_arr = match args
        .get(2)
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::arg_array(ctx, a))
    {
        Ok(a) => a,
        Err(e) => return err(e),
    };
    let if_not_found = match args::opt_scalar(ctx, args, 3) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let match_mode = match args::opt_number(ctx, args, 4, 0.0).and_then(args::trunc_i64) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let search_mode = match args::opt_number(ctx, args, 5, 1.0).and_then(args::trunc_i64) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let keys = match args::as_vector(&lookup_arr) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let n = keys.len();
    let by_row = lookup_arr.cols == 1;
    if by_row {
        if return_arr.rows as usize != n {
            return err(ErrorKind::Value);
        }
    } else if return_arr.cols as usize != n {
        return err(ErrorKind::Value);
    }
    let idx = match find_x(&keys, &lookup, match_mode, search_mode) {
        Ok(Some(i)) => i,
        Ok(None) => {
            return match if_not_found {
                Some(s) => RuntimeValue::Scalar(s),
                None => err(ErrorKind::Na),
            };
        }
        Err(e) => return err(e),
    };
    take_return(&return_arr, idx, by_row)
}

fn xmatch_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let lookup = match args.first() {
        Some(a) => match args::scalar(ctx, a) {
            Ok(s) => s,
            Err(e) => return err(e),
        },
        None => return err(ErrorKind::Value),
    };
    let lookup_arr = match args
        .get(1)
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::arg_array(ctx, a))
    {
        Ok(a) => a,
        Err(e) => return err(e),
    };
    let match_mode = match args::opt_number(ctx, args, 2, 0.0).and_then(args::trunc_i64) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let search_mode = match args::opt_number(ctx, args, 3, 1.0).and_then(args::trunc_i64) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let keys = match args::as_vector(&lookup_arr) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    match find_x(&keys, &lookup, match_mode, search_mode) {
        Ok(Some(i)) => RuntimeValue::Scalar(Scalar::Number((i + 1) as f64)),
        Ok(None) => err(ErrorKind::Na),
        Err(e) => err(e),
    }
}

fn find_x(
    keys: &[Scalar],
    lookup: &Scalar,
    match_mode: i64,
    search_mode: i64,
) -> Result<Option<usize>, ErrorKind> {
    if !(-1..=2).contains(&match_mode) {
        return Err(ErrorKind::Value);
    }
    if !matches!(search_mode, -2 | -1 | 1 | 2) {
        return Err(ErrorKind::Value);
    }
    let wildcard = match_mode == 2;
    let approx_small = match_mode == -1;
    let approx_large = match_mode == 1;
    match search_mode {
        1 => scan_x(keys, lookup, wildcard, approx_small, approx_large, false),
        -1 => scan_x(keys, lookup, wildcard, approx_small, approx_large, true),
        2 => binary_x(keys, lookup, match_mode, true),
        -2 => binary_x(keys, lookup, match_mode, false),
        _ => Err(ErrorKind::Value),
    }
}

fn scan_x(
    keys: &[Scalar],
    lookup: &Scalar,
    wildcard: bool,
    approx_small: bool,
    approx_large: bool,
    reverse: bool,
) -> Result<Option<usize>, ErrorKind> {
    let n = keys.len();
    let iter: Box<dyn Iterator<Item = usize>> = if reverse {
        Box::new((0..n).rev())
    } else {
        Box::new(0..n)
    };
    let mut best: Option<usize> = None;
    for i in iter {
        if wildcard {
            let p = args::as_text(lookup)?;
            let t = args::as_text(&keys[i])?;
            if args::wildcard_eq(&p, &t) {
                return Ok(Some(i));
            }
            continue;
        }
        if args::exact_eq(lookup, &keys[i]) {
            return Ok(Some(i));
        }
        if (approx_small || approx_large)
            && let Ok(c) = coerce::compare(&keys[i], lookup)
        {
            let ok = if approx_small {
                matches!(
                    c,
                    omacell_core::coerce::Cmp::Lt | omacell_core::coerce::Cmp::Eq
                )
            } else {
                matches!(
                    c,
                    omacell_core::coerce::Cmp::Gt | omacell_core::coerce::Cmp::Eq
                )
            };
            if ok {
                let replace = match best {
                    None => true,
                    Some(b) => {
                        let cb = coerce::compare(&keys[i], &keys[b])?;
                        if approx_small {
                            matches!(cb, omacell_core::coerce::Cmp::Gt)
                        } else {
                            matches!(cb, omacell_core::coerce::Cmp::Lt)
                        }
                    }
                };
                if replace {
                    best = Some(i);
                }
            }
        }
    }
    Ok(best)
}

fn binary_x(
    keys: &[Scalar],
    lookup: &Scalar,
    match_mode: i64,
    ascending: bool,
) -> Result<Option<usize>, ErrorKind> {
    if match_mode == 2 {
        return scan_x(keys, lookup, true, false, false, !ascending);
    }
    if ascending {
        match match_mode {
            0 => {
                let Some(i) = args::last_le(keys, lookup)? else {
                    return Ok(None);
                };
                if args::exact_eq(&keys[i], lookup) {
                    Ok(Some(i))
                } else {
                    Ok(None)
                }
            }
            -1 => args::last_le(keys, lookup),
            1 => {
                // first >= lookup
                let le = args::last_le(keys, lookup)?;
                if let Some(i) = le {
                    if args::exact_eq(&keys[i], lookup) {
                        return Ok(Some(i));
                    }
                    let next = i + 1;
                    if next < keys.len() {
                        return Ok(Some(next));
                    }
                    return Ok(None);
                }
                if keys.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(0))
                }
            }
            _ => Err(ErrorKind::Value),
        }
    } else {
        match match_mode {
            0 => {
                let Some(i) = args::last_ge_desc(keys, lookup)? else {
                    return Ok(None);
                };
                if args::exact_eq(&keys[i], lookup) {
                    Ok(Some(i))
                } else {
                    Ok(None)
                }
            }
            1 => args::last_ge_desc(keys, lookup),
            -1 => {
                let ge = args::last_ge_desc(keys, lookup)?;
                if let Some(i) = ge {
                    if args::exact_eq(&keys[i], lookup) {
                        return Ok(Some(i));
                    }
                    let next = i + 1;
                    if next < keys.len() {
                        return Ok(Some(next));
                    }
                    return Ok(None);
                }
                if keys.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(0))
                }
            }
            _ => Err(ErrorKind::Value),
        }
    }
}

fn take_return(array: &RuntimeArray, idx: usize, by_row: bool) -> RuntimeValue {
    if by_row {
        let row = idx as u32;
        if array.cols == 1 {
            return RuntimeValue::Scalar(args::at(array, row, 0));
        }
        let mut values = Vec::with_capacity(array.cols as usize);
        for c in 0..array.cols {
            values.push(args::at(array, row, c));
        }
        args::array_result(1, array.cols, values)
    } else {
        let col = idx as u32;
        if array.rows == 1 {
            return RuntimeValue::Scalar(args::at(array, 0, col));
        }
        let mut values = Vec::with_capacity(array.rows as usize);
        for r in 0..array.rows {
            values.push(args::at(array, r, col));
        }
        args::array_result(array.rows, 1, values)
    }
}

fn index_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let Some(first) = args.first() else {
        return err(ErrorKind::Value);
    };
    if let Some(e) = first.value.error_kind() {
        return err(e);
    }
    let area_num = match args::opt_number(ctx, args, 3, 1.0).and_then(args::trunc_i64) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    if let RuntimeValue::Ref(r) = &first.value {
        let area = match args::nth_area(r, area_num) {
            Ok(a) => a.clone(),
            Err(e) => return err(e),
        };
        return index_ref(ctx, &area, args);
    }
    let array = match args::arg_array(ctx, first) {
        Ok(a) => a,
        Err(e) => return err(e),
    };
    index_array(&array, args, ctx)
}

fn index_array(array: &RuntimeArray, args: &[ArgVal], ctx: &mut EvalCtx<'_>) -> RuntimeValue {
    let row = match row_col_arg(ctx, args, 1) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let col = match row_col_arg(ctx, args, 2) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    pick_array(array, row, col)
}

fn row_col_arg(
    ctx: &mut EvalCtx<'_>,
    args: &[ArgVal],
    index: usize,
) -> Result<Option<i64>, ErrorKind> {
    match args::opt_scalar(ctx, args, index)? {
        None => Ok(None),
        Some(s) => {
            let n = args::trunc_i64(coerce::to_number(&s)?)?;
            Ok(Some(n))
        }
    }
}

fn pick_array(array: &RuntimeArray, row: Option<i64>, col: Option<i64>) -> RuntimeValue {
    let is_vector = array.rows == 1 || array.cols == 1;
    let (row, col) = if is_vector && col.is_none() {
        if array.rows == 1 {
            (Some(1), row)
        } else {
            (row, Some(1))
        }
    } else {
        (row, col)
    };
    let r = match decode_index(row, array.rows) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let c = match decode_index(col, array.cols) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    match (r, c) {
        (None, None) => {
            let values = array.values.iter().cloned().collect();
            args::array_result(array.rows, array.cols, values)
        }
        (None, Some(c)) => {
            let mut values = Vec::with_capacity(array.rows as usize);
            for rr in 0..array.rows {
                values.push(args::at(array, rr, c));
            }
            args::array_result(array.rows, 1, values)
        }
        (Some(r), None) => {
            let mut values = Vec::with_capacity(array.cols as usize);
            for cc in 0..array.cols {
                values.push(args::at(array, r, cc));
            }
            args::array_result(1, array.cols, values)
        }
        (Some(r), Some(c)) => RuntimeValue::Scalar(args::at(array, r, c)),
    }
}

fn decode_index(idx: Option<i64>, n: u32) -> Result<Option<u32>, ErrorKind> {
    match idx {
        None | Some(0) => Ok(None),
        Some(v) if v < 0 => Err(ErrorKind::Value),
        Some(v) => {
            let u = u32::try_from(v).map_err(|_| ErrorKind::Ref)?;
            if u > n {
                Err(ErrorKind::Ref)
            } else {
                Ok(Some(u - 1))
            }
        }
    }
}

fn index_ref(ctx: &mut EvalCtx<'_>, r: &Reference, args: &[ArgVal]) -> RuntimeValue {
    let (rows, cols) = match args::reference_shape(r) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let row = match row_col_arg(ctx, args, 1) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let col = match row_col_arg(ctx, args, 2) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let Reference::Range {
        sheet,
        start_row,
        start_col,
        end_row,
        end_col,
    } = r
    else {
        let array = match args::to_array(ctx, RuntimeValue::Ref(r.clone())) {
            Ok(a) => a,
            Err(e) => return err(e),
        };
        return pick_array(&array, row, col);
    };
    let r1 = (*start_row).min(*end_row);
    let c1 = (*start_col).min(*end_col);
    let is_vector = rows == 1 || cols == 1;
    let (row, col) = if is_vector && col.is_none() {
        if rows == 1 {
            (Some(1), row)
        } else {
            (row, Some(1))
        }
    } else {
        (row, col)
    };
    let rr = match decode_index(row, rows) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let cc = match decode_index(col, cols) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let out = match (rr, cc) {
        (None, None) => r.clone(),
        (None, Some(c)) => Reference::Range {
            sheet: *sheet,
            start_row: r1,
            start_col: c1 + c as u16,
            end_row: r1 + rows - 1,
            end_col: c1 + c as u16,
        },
        (Some(row), None) => Reference::Range {
            sheet: *sheet,
            start_row: r1 + row,
            start_col: c1,
            end_row: r1 + row,
            end_col: c1 + cols as u16 - 1,
        },
        (Some(row), Some(c)) => args::cell_ref(*sheet, r1 + row, c1 + c as u16),
    };
    RuntimeValue::Ref(out)
}

fn match_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let lookup = match args.first() {
        Some(a) => match args::scalar(ctx, a) {
            Ok(s) => s,
            Err(e) => return err(e),
        },
        None => return err(ErrorKind::Value),
    };
    let array = match args
        .get(1)
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::arg_array(ctx, a))
    {
        Ok(a) => a,
        Err(e) => return err(e),
    };
    let match_type = match args::opt_number(ctx, args, 2, 1.0).and_then(args::trunc_i64) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let keys = match args::as_vector(&array) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let found = match match_type {
        0 => {
            let mut hit = None;
            for (i, k) in keys.iter().enumerate() {
                if args::exact_eq(&lookup, k) {
                    hit = Some(i);
                    break;
                }
                if let (Ok(p), Ok(t)) = (args::as_text(&lookup), args::as_text(k))
                    && (p.contains('*') || p.contains('?') || p.contains('~'))
                    && args::wildcard_eq(&p, &t)
                {
                    hit = Some(i);
                    break;
                }
            }
            hit
        }
        1 => match args::last_le(&keys, &lookup) {
            Ok(v) => v,
            Err(e) => return err(e),
        },
        -1 => match args::last_ge_desc(&keys, &lookup) {
            Ok(v) => v,
            Err(e) => return err(e),
        },
        _ => return err(ErrorKind::Na),
    };
    match found {
        Some(i) => RuntimeValue::Scalar(Scalar::Number((i + 1) as f64)),
        None => err(ErrorKind::Na),
    }
}

fn vlookup_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    table_lookup(ctx, args, true)
}

fn hlookup_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    table_lookup(ctx, args, false)
}

fn table_lookup(ctx: &mut EvalCtx<'_>, args: &[ArgVal], vertical: bool) -> RuntimeValue {
    let lookup = match args.first() {
        Some(a) => match args::scalar(ctx, a) {
            Ok(s) => s,
            Err(e) => return err(e),
        },
        None => return err(ErrorKind::Value),
    };
    let table = match args
        .get(1)
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::arg_array(ctx, a))
    {
        Ok(a) => a,
        Err(e) => return err(e),
    };
    let index = match args
        .get(2)
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::number(ctx, a))
    {
        Ok(n) => match args::trunc_i64(n) {
            Ok(v) => v,
            Err(e) => return err(e),
        },
        Err(e) => return err(e),
    };
    if index < 1 {
        return err(ErrorKind::Value);
    }
    let range_lookup = match args::opt_bool(ctx, args, 3, true) {
        Ok(b) => b,
        Err(e) => return err(e),
    };
    let (nkeys, breadth) = if vertical {
        (table.rows as usize, table.cols)
    } else {
        (table.cols as usize, table.rows)
    };
    if index as u32 > breadth {
        return err(ErrorKind::Ref);
    }
    let mut keys = Vec::with_capacity(nkeys);
    for i in 0..nkeys {
        keys.push(if vertical {
            args::at(&table, i as u32, 0)
        } else {
            args::at(&table, 0, i as u32)
        });
    }
    let found = if range_lookup {
        match args::last_le(&keys, &lookup) {
            Ok(v) => v,
            Err(e) => return err(e),
        }
    } else {
        let mut hit = None;
        for (i, k) in keys.iter().enumerate() {
            if args::exact_eq(&lookup, k) {
                hit = Some(i);
                break;
            }
            if let (Ok(p), Ok(t)) = (args::as_text(&lookup), args::as_text(k))
                && (p.contains('*') || p.contains('?') || p.contains('~'))
                && args::wildcard_eq(&p, &t)
            {
                hit = Some(i);
                break;
            }
        }
        hit
    };
    let Some(i) = found else {
        return err(ErrorKind::Na);
    };
    let ret = if vertical {
        args::at(&table, i as u32, (index as u32) - 1)
    } else {
        args::at(&table, (index as u32) - 1, i as u32)
    };
    RuntimeValue::Scalar(ret)
}

fn lookup_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let lookup = match args.first() {
        Some(a) => match args::scalar(ctx, a) {
            Ok(s) => s,
            Err(e) => return err(e),
        },
        None => return err(ErrorKind::Value),
    };
    let second = match args
        .get(1)
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::arg_array(ctx, a))
    {
        Ok(a) => a,
        Err(e) => return err(e),
    };
    let (keys, results) = if let Some(third) = args.get(2) {
        if third.omitted {
            array_form(&second)
        } else {
            let res = match args::arg_array(ctx, third) {
                Ok(a) => a,
                Err(e) => return err(e),
            };
            let keys = match args::as_vector(&second) {
                Ok(v) => v,
                Err(e) => return err(e),
            };
            let results = match args::as_vector(&res) {
                Ok(v) => v,
                Err(e) => return err(e),
            };
            (keys, results)
        }
    } else {
        array_form(&second)
    };
    match args::last_le(&keys, &lookup) {
        Ok(Some(i)) => match results.get(i) {
            Some(s) => RuntimeValue::Scalar(s.clone()),
            None => err(ErrorKind::Na),
        },
        Ok(None) => err(ErrorKind::Na),
        Err(e) => err(e),
    }
}

fn array_form(array: &RuntimeArray) -> (Vec<Scalar>, Vec<Scalar>) {
    if array.rows >= array.cols {
        // search first column, return last column
        let mut keys = Vec::with_capacity(array.rows as usize);
        let mut results = Vec::with_capacity(array.rows as usize);
        for r in 0..array.rows {
            keys.push(args::at(array, r, 0));
            results.push(args::at(array, r, array.cols - 1));
        }
        (keys, results)
    } else {
        let mut keys = Vec::with_capacity(array.cols as usize);
        let mut results = Vec::with_capacity(array.cols as usize);
        for c in 0..array.cols {
            keys.push(args::at(array, 0, c));
            results.push(args::at(array, array.rows - 1, c));
        }
        (keys, results)
    }
}

fn choose_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let Some(first) = args.first() else {
        return err(ErrorKind::Value);
    };
    let idx = match args::number(ctx, first).and_then(args::trunc_i64) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    if idx < 1 {
        return err(ErrorKind::Value);
    }
    let pos = usize::try_from(idx).unwrap_or(usize::MAX);
    match args.get(pos) {
        Some(a) => a.value.clone(),
        None => err(ErrorKind::Value),
    }
}

fn offset_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let Some(first) = args.first() else {
        return err(ErrorKind::Value);
    };
    if let Some(e) = first.value.error_kind() {
        return err(e);
    }
    let r = match &first.value {
        RuntimeValue::Ref(r) => r.clone(),
        RuntimeValue::Scalar(Scalar::Error(e)) => return err(*e),
        _ => return err(ErrorKind::Value),
    };
    let rows = match args
        .get(1)
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::number(ctx, a))
    {
        Ok(n) => match args::trunc_i64(n) {
            Ok(v) => v,
            Err(e) => return err(e),
        },
        Err(e) => return err(e),
    };
    let cols = match args
        .get(2)
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::number(ctx, a))
    {
        Ok(n) => match args::trunc_i64(n) {
            Ok(v) => v,
            Err(e) => return err(e),
        },
        Err(e) => return err(e),
    };
    let height = match args::opt_scalar(ctx, args, 3) {
        Ok(None) => None,
        Ok(Some(s)) => match coerce::to_number(&s).and_then(args::trunc_i64) {
            Ok(v) => Some(v),
            Err(e) => return err(e),
        },
        Err(e) => return err(e),
    };
    let width = match args::opt_scalar(ctx, args, 4) {
        Ok(None) => None,
        Ok(Some(s)) => match coerce::to_number(&s).and_then(args::trunc_i64) {
            Ok(v) => Some(v),
            Err(e) => return err(e),
        },
        Err(e) => return err(e),
    };
    match args::offset_ref(&r, rows, cols, height, width) {
        Ok(out) => {
            ctx.record_dynamic_ref(out.clone());
            RuntimeValue::Ref(out)
        }
        Err(e) => err(e),
    }
}

fn indirect_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let Some(first) = args.first() else {
        return err(ErrorKind::Value);
    };
    let text = match args::scalar(ctx, first).and_then(|s| args::as_text(&s)) {
        Ok(t) => t,
        Err(e) => return err(e),
    };
    let a1 = match args::opt_bool(ctx, args, 1, true) {
        Ok(b) => b,
        Err(e) => return err(e),
    };
    let cell = ctx.coord();
    match args::parse_address(ctx.workbook(), &text, a1, cell.row, cell.col, cell.sheet) {
        Ok(r) => {
            ctx.record_dynamic_ref(r.clone());
            RuntimeValue::Ref(r)
        }
        Err(_) => err(ErrorKind::Ref),
    }
}

fn row_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    if args::is_omitted(args, 0) {
        return RuntimeValue::Scalar(Scalar::Number(f64::from(ctx.coord().row + 1)));
    }
    let Some(first) = args.first() else {
        return RuntimeValue::Scalar(Scalar::Number(f64::from(ctx.coord().row + 1)));
    };
    match &first.value {
        RuntimeValue::Scalar(Scalar::Error(e)) => err(*e),
        RuntimeValue::Ref(r) => row_from_ref(r),
        other => match args::to_array(ctx, other.clone()) {
            Ok(a) => {
                // Array literals have no sheet coordinates; treat as starting at 1.
                let mut values = Vec::with_capacity(a.rows as usize);
                for i in 0..a.rows {
                    values.push(Scalar::Number(f64::from(i + 1)));
                }
                args::array_result(a.rows, 1, values)
            }
            Err(e) => err(e),
        },
    }
}

fn row_from_ref(r: &Reference) -> RuntimeValue {
    match r {
        Reference::Range {
            start_row, end_row, ..
        } => {
            let r1 = (*start_row).min(*end_row);
            let r2 = (*start_row).max(*end_row);
            if r1 == r2 {
                return RuntimeValue::Scalar(Scalar::Number(f64::from(r1 + 1)));
            }
            let rows = r2 - r1 + 1;
            let Ok(len) = args::check_shape(rows, 1) else {
                return err(ErrorKind::Num);
            };
            let mut values = Vec::with_capacity(len);
            for row in r1..=r2 {
                values.push(Scalar::Number(f64::from(row + 1)));
            }
            args::array_result(rows, 1, values)
        }
        Reference::Union(parts) if parts.len() == 1 => row_from_ref(&parts[0]),
        _ => err(ErrorKind::Value),
    }
}

fn column_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    if args::is_omitted(args, 0) {
        return RuntimeValue::Scalar(Scalar::Number(f64::from(u32::from(ctx.coord().col) + 1)));
    }
    let Some(first) = args.first() else {
        return RuntimeValue::Scalar(Scalar::Number(f64::from(u32::from(ctx.coord().col) + 1)));
    };
    match &first.value {
        RuntimeValue::Scalar(Scalar::Error(e)) => err(*e),
        RuntimeValue::Ref(r) => col_from_ref(r),
        other => match args::to_array(ctx, other.clone()) {
            Ok(a) => {
                let mut values = Vec::with_capacity(a.cols as usize);
                for i in 0..a.cols {
                    values.push(Scalar::Number(f64::from(i + 1)));
                }
                args::array_result(1, a.cols, values)
            }
            Err(e) => err(e),
        },
    }
}

fn col_from_ref(r: &Reference) -> RuntimeValue {
    match r {
        Reference::Range {
            start_col, end_col, ..
        } => {
            let c1 = (*start_col).min(*end_col);
            let c2 = (*start_col).max(*end_col);
            if c1 == c2 {
                return RuntimeValue::Scalar(Scalar::Number(f64::from(u32::from(c1) + 1)));
            }
            let cols = u32::from(c2 - c1) + 1;
            let Ok(len) = args::check_shape(1, cols) else {
                return err(ErrorKind::Num);
            };
            let mut values = Vec::with_capacity(len);
            for col in c1..=c2 {
                values.push(Scalar::Number(f64::from(u32::from(col) + 1)));
            }
            args::array_result(1, cols, values)
        }
        Reference::Union(parts) if parts.len() == 1 => col_from_ref(&parts[0]),
        _ => err(ErrorKind::Value),
    }
}

fn rows_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let Some(first) = args.first() else {
        return err(ErrorKind::Value);
    };
    if let Some(e) = first.value.error_kind() {
        return err(e);
    }
    match &first.value {
        RuntimeValue::Ref(r) => match args::reference_shape(r) {
            Ok((rows, _)) => RuntimeValue::Scalar(Scalar::Number(f64::from(rows))),
            Err(e) => err(e),
        },
        other => match args::to_array(ctx, other.clone()) {
            Ok(a) => RuntimeValue::Scalar(Scalar::Number(f64::from(a.rows))),
            Err(e) => err(e),
        },
    }
}

fn columns_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let Some(first) = args.first() else {
        return err(ErrorKind::Value);
    };
    if let Some(e) = first.value.error_kind() {
        return err(e);
    }
    match &first.value {
        RuntimeValue::Ref(r) => match args::reference_shape(r) {
            Ok((_, cols)) => RuntimeValue::Scalar(Scalar::Number(f64::from(cols))),
            Err(e) => err(e),
        },
        other => match args::to_array(ctx, other.clone()) {
            Ok(a) => RuntimeValue::Scalar(Scalar::Number(f64::from(a.cols))),
            Err(e) => err(e),
        },
    }
}

fn address_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let row = match args
        .first()
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::number(ctx, a))
    {
        Ok(n) => match args::trunc_i64(n) {
            Ok(v) => v,
            Err(e) => return err(e),
        },
        Err(e) => return err(e),
    };
    let col = match args
        .get(1)
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::number(ctx, a))
    {
        Ok(n) => match args::trunc_i64(n) {
            Ok(v) => v,
            Err(e) => return err(e),
        },
        Err(e) => return err(e),
    };
    if row < 1 || col < 1 {
        return err(ErrorKind::Value);
    }
    if row > i64::from(MAX_ROWS) || col > i64::from(MAX_COLS) {
        return err(ErrorKind::Value);
    }
    let abs_num = match args::opt_number(ctx, args, 2, 1.0).and_then(args::trunc_i64) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    if !(1..=4).contains(&abs_num) {
        return err(ErrorKind::Value);
    }
    let a1 = match args::opt_bool(ctx, args, 3, true) {
        Ok(b) => b,
        Err(e) => return err(e),
    };
    let sheet = match args::opt_scalar(ctx, args, 4) {
        Ok(Some(s)) => match coerce::to_text(&s) {
            Ok(t) => Some(t.to_string()),
            Err(e) => return err(e),
        },
        Ok(None) => None,
        Err(e) => return err(e),
    };
    let col_u = (col as u32 - 1) as u16;
    let row_u = (row as u32) - 1;
    let col_abs = abs_num == 1 || abs_num == 3;
    let row_abs = abs_num == 1 || abs_num == 2;
    let body = if a1 {
        let letters = match col_to_letters(col_u) {
            Ok(s) => s,
            Err(_) => return err(ErrorKind::Value),
        };
        let mut s = String::new();
        if col_abs {
            s.push('$');
        }
        s.push_str(&letters);
        if row_abs {
            s.push('$');
        }
        s.push_str(&(row_u + 1).to_string());
        s
    } else {
        match abs_num {
            1 => format!("R{}C{}", row_u + 1, col_u as u32 + 1),
            2 => format!("R{}C[{}]", row_u + 1, col_u as u32 + 1),
            3 => format!("R[{}]C{}", row_u + 1, col_u as u32 + 1),
            _ => format!("R[{}]C[{}]", row_u + 1, col_u as u32 + 1),
        }
    };
    let text = match sheet {
        Some(name) => format!("{}!{body}", quote_sheet_name(&name)),
        None => body,
    };
    RuntimeValue::Scalar(Scalar::Text(text.into()))
}

fn areas_impl(_ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let Some(first) = args.first() else {
        return err(ErrorKind::Value);
    };
    match &first.value {
        RuntimeValue::Scalar(Scalar::Error(e)) => err(*e),
        RuntimeValue::Ref(r) => {
            RuntimeValue::Scalar(Scalar::Number(f64::from(args::area_count(r))))
        }
        RuntimeValue::Array(_) | RuntimeValue::Scalar(_) => {
            RuntimeValue::Scalar(Scalar::Number(1.0))
        }
        RuntimeValue::Lambda(_) => err(ErrorKind::Value),
    }
}
