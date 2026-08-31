//! Dynamic array functions (WP-05c). Replaces the WP-05F `SEQUENCE` probe.

use std::collections::HashMap;

use omacell_core::coerce::{self, Cmp, Scalar};
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, FnBody, FnRegistry, RuntimeArray, RuntimeValue};
use omacell_core::limits::{MAX_COLS, MAX_ROWS};

use crate::args;
use crate::metadata::{ArgKind, ArrayBehavior, FunctionSpec};

/// Array-manipulation specs in declaration order.
pub const SPECS: &[FunctionSpec] = &[
    TRANSPOSE, FILTER, SORT, SORTBY, UNIQUE, SEQUENCE, RANDARRAY, TAKE, DROP, CHOOSEROWS,
    CHOOSECOLS, VSTACK, HSTACK, TOCOL, TOROW, WRAPROWS, WRAPCOLS, EXPAND,
];

/// Register array functions (including `SEQUENCE`, replacing the probe).
pub fn register_array(registry: &mut FnRegistry) {
    for spec in SPECS {
        args::register_spec(registry, spec);
    }
}

crate::define_fn! {
const TRANSPOSE = {
    name: "TRANSPOSE",
    aliases: &[],
    tier: 0,
    category: "array",
    arg_kinds: &[ArgKind::Array],
    min_args: 1,
    max_args: 1,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "TRANSPOSE(array)",
    doc: "Transposes a vertical range to horizontal, or vice versa.",
    body: FnBody::Eager(transpose_impl),
};
}

crate::define_fn! {
const FILTER = {
    name: "FILTER",
    aliases: &[],
    tier: 0,
    category: "array",
    arg_kinds: &[ArgKind::Array, ArgKind::Array, ArgKind::Any],
    min_args: 2,
    max_args: 3,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "FILTER(array, include, [if_empty])",
    doc: "Filters an array by a boolean mask. No keepers without if_empty is `#CALC!`.",
    body: FnBody::Eager(filter_impl),
};
}

crate::define_fn! {
const SORT = {
    name: "SORT",
    aliases: &[],
    tier: 0,
    category: "array",
    arg_kinds: &[ArgKind::Array, ArgKind::Number, ArgKind::Number, ArgKind::Logical],
    min_args: 1,
    max_args: 4,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "SORT(array, [sort_index], [sort_order], [by_col])",
    doc: "Sorts an array by a row or column. Equal keys keep original order.",
    body: FnBody::Eager(sort_impl),
};
}

crate::define_fn! {
const SORTBY = {
    name: "SORTBY",
    aliases: &[],
    tier: 0,
    category: "array",
    arg_kinds: &[ArgKind::Array, ArgKind::Array, ArgKind::Number],
    min_args: 2,
    max_args: 255,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "SORTBY(array, by_array1, [sort_order1], ...)",
    doc: "Sorts an array by one or more parallel key arrays. Stable.",
    body: FnBody::Eager(sortby_impl),
};
}

crate::define_fn! {
const UNIQUE = {
    name: "UNIQUE",
    aliases: &[],
    tier: 0,
    category: "array",
    arg_kinds: &[ArgKind::Array, ArgKind::Logical, ArgKind::Logical],
    min_args: 1,
    max_args: 3,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "UNIQUE(array, [by_col], [exactly_once])",
    doc: "Returns unique rows (or columns) of an array.",
    body: FnBody::Eager(unique_impl),
};
}

crate::define_fn! {
const SEQUENCE = {
    name: "SEQUENCE",
    aliases: &[],
    tier: 0,
    category: "array",
    arg_kinds: &[ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number],
    min_args: 1,
    max_args: 4,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "SEQUENCE(rows, [columns], [start], [step])",
    doc: "Returns a sequence array. Invalid or out-of-grid shapes are `#NUM!` before allocation.",
    body: FnBody::Eager(sequence_impl),
};
}

crate::define_fn! {
const RANDARRAY = {
    name: "RANDARRAY",
    aliases: &[],
    tier: 0,
    category: "array",
    arg_kinds: &[ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Logical],
    min_args: 0,
    max_args: 5,
    volatile: true,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "RANDARRAY([rows], [columns], [min], [max], [integer])",
    doc: "Returns an array of random numbers derived from the pass nonce. Volatile.",
    body: FnBody::Eager(randarray_impl),
};
}

crate::define_fn! {
const TAKE = {
    name: "TAKE",
    aliases: &[],
    tier: 0,
    category: "array",
    arg_kinds: &[ArgKind::Array, ArgKind::Number, ArgKind::Number],
    min_args: 2,
    max_args: 3,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "TAKE(array, rows, [columns])",
    doc: "Takes the first or last rows/columns of an array. Empty result is `#CALC!`.",
    body: FnBody::Eager(take_impl),
};
}

crate::define_fn! {
const DROP = {
    name: "DROP",
    aliases: &[],
    tier: 0,
    category: "array",
    arg_kinds: &[ArgKind::Array, ArgKind::Number, ArgKind::Number],
    min_args: 2,
    max_args: 3,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "DROP(array, rows, [columns])",
    doc: "Drops the first or last rows/columns of an array. Empty result is `#CALC!`.",
    body: FnBody::Eager(drop_impl),
};
}

crate::define_fn! {
const CHOOSEROWS = {
    name: "CHOOSEROWS",
    aliases: &[],
    tier: 0,
    category: "array",
    arg_kinds: &[ArgKind::Array, ArgKind::Number],
    min_args: 2,
    max_args: 255,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "CHOOSEROWS(array, row_num1, [row_num2], ...)",
    doc: "Returns the specified rows of an array. Negative indices count from the end.",
    body: FnBody::Eager(chooserows_impl),
};
}

crate::define_fn! {
const CHOOSECOLS = {
    name: "CHOOSECOLS",
    aliases: &[],
    tier: 0,
    category: "array",
    arg_kinds: &[ArgKind::Array, ArgKind::Number],
    min_args: 2,
    max_args: 255,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "CHOOSECOLS(array, col_num1, [col_num2], ...)",
    doc: "Returns the specified columns of an array. Negative indices count from the end.",
    body: FnBody::Eager(choosecols_impl),
};
}

crate::define_fn! {
const VSTACK = {
    name: "VSTACK",
    aliases: &[],
    tier: 0,
    category: "array",
    arg_kinds: &[ArgKind::Array],
    min_args: 1,
    max_args: 255,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "VSTACK(array1, [array2], ...)",
    doc: "Appends arrays vertically. Shape is checked before allocation.",
    body: FnBody::Eager(vstack_impl),
};
}

crate::define_fn! {
const HSTACK = {
    name: "HSTACK",
    aliases: &[],
    tier: 0,
    category: "array",
    arg_kinds: &[ArgKind::Array],
    min_args: 1,
    max_args: 255,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "HSTACK(array1, [array2], ...)",
    doc: "Appends arrays horizontally. Shape is checked before allocation.",
    body: FnBody::Eager(hstack_impl),
};
}

crate::define_fn! {
const TOCOL = {
    name: "TOCOL",
    aliases: &[],
    tier: 0,
    category: "array",
    arg_kinds: &[ArgKind::Array, ArgKind::Number, ArgKind::Logical],
    min_args: 1,
    max_args: 3,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "TOCOL(array, [ignore], [scan_by_column])",
    doc: "Flattens an array into a column.",
    body: FnBody::Eager(tocol_impl),
};
}

crate::define_fn! {
const TOROW = {
    name: "TOROW",
    aliases: &[],
    tier: 0,
    category: "array",
    arg_kinds: &[ArgKind::Array, ArgKind::Number, ArgKind::Logical],
    min_args: 1,
    max_args: 3,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "TOROW(array, [ignore], [scan_by_column])",
    doc: "Flattens an array into a row.",
    body: FnBody::Eager(torow_impl),
};
}

crate::define_fn! {
const WRAPROWS = {
    name: "WRAPROWS",
    aliases: &[],
    tier: 0,
    category: "array",
    arg_kinds: &[ArgKind::Array, ArgKind::Number, ArgKind::Any],
    min_args: 2,
    max_args: 3,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "WRAPROWS(vector, wrap_count, [pad_with])",
    doc: "Wraps a vector into rows of `wrap_count` columns. Invalid wrap_count is `#NUM!` before allocation.",
    body: FnBody::Eager(wraprows_impl),
};
}

crate::define_fn! {
const WRAPCOLS = {
    name: "WRAPCOLS",
    aliases: &[],
    tier: 0,
    category: "array",
    arg_kinds: &[ArgKind::Array, ArgKind::Number, ArgKind::Any],
    min_args: 2,
    max_args: 3,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "WRAPCOLS(vector, wrap_count, [pad_with])",
    doc: "Wraps a vector into columns of `wrap_count` rows. Invalid wrap_count is `#NUM!` before allocation.",
    body: FnBody::Eager(wrapcols_impl),
};
}

crate::define_fn! {
const EXPAND = {
    name: "EXPAND",
    aliases: &[],
    tier: 0,
    category: "array",
    arg_kinds: &[ArgKind::Array, ArgKind::Number, ArgKind::Number, ArgKind::Any],
    min_args: 2,
    max_args: 4,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "EXPAND(array, rows, [columns], [pad_with])",
    doc: "Expands an array to a larger shape, padding with `#N/A` by default.",
    body: FnBody::Eager(expand_impl),
};
}

fn err(e: ErrorKind) -> RuntimeValue {
    RuntimeValue::error(e)
}

fn transpose_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let Some(first) = args.first() else {
        return err(ErrorKind::Value);
    };
    if let Some(e) = first.value.error_kind() {
        return err(e);
    }
    let array = match args::arg_array(ctx, first) {
        Ok(a) => a,
        Err(e) => return err(e),
    };
    let Ok(len) = args::check_shape(array.cols, array.rows) else {
        return err(ErrorKind::Num);
    };
    let mut values = Vec::with_capacity(len);
    for c in 0..array.cols {
        for r in 0..array.rows {
            values.push(args::at(&array, r, c));
        }
    }
    args::array_result(array.cols, array.rows, values)
}

fn filter_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let array = match args
        .first()
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::arg_array(ctx, a))
    {
        Ok(a) => a,
        Err(e) => return err(e),
    };
    let include = match args
        .get(1)
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::arg_array(ctx, a))
    {
        Ok(a) => a,
        Err(e) => return err(e),
    };
    let if_empty = match args::opt_scalar(ctx, args, 2) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let by_row = include.cols == 1 || (include.rows == array.rows && include.cols != array.cols);
    let (n, keep) = if include.rows == array.rows
        && (include.cols == 1 || include.cols == array.cols && array.cols == 1)
    {
        (array.rows, true)
    } else if include.cols == array.cols && include.rows == 1 {
        (array.cols, false)
    } else if include.rows == array.rows && include.cols == 1 {
        (array.rows, true)
    } else {
        return err(ErrorKind::Value);
    };
    let by_row = keep || by_row;
    let mut mask = Vec::with_capacity(n as usize);
    for i in 0..n {
        let s = if by_row {
            args::at(&include, i, 0)
        } else {
            args::at(&include, 0, i)
        };
        if let Some(e) = s.error() {
            return err(e);
        }
        match coerce::to_bool(&s) {
            Ok(b) => mask.push(b),
            Err(_) => match coerce::to_number(&s) {
                Ok(v) => mask.push(v != 0.0),
                Err(e) => return err(e),
            },
        }
    }
    let kept: Vec<u32> = mask
        .iter()
        .enumerate()
        .filter_map(|(i, k)| k.then_some(i as u32))
        .collect();
    if kept.is_empty() {
        return match if_empty {
            Some(s) => RuntimeValue::Scalar(s),
            None => args::empty_array(),
        };
    }
    if by_row {
        let rows = kept.len() as u32;
        let cols = array.cols;
        let Ok(len) = args::check_shape(rows, cols) else {
            return err(ErrorKind::Num);
        };
        let mut values = Vec::with_capacity(len);
        for r in kept {
            for c in 0..cols {
                values.push(args::at(&array, r, c));
            }
        }
        args::array_result(rows, cols, values)
    } else {
        let cols = kept.len() as u32;
        let rows = array.rows;
        let Ok(len) = args::check_shape(rows, cols) else {
            return err(ErrorKind::Num);
        };
        let mut values = Vec::with_capacity(len);
        for r in 0..rows {
            for c in &kept {
                values.push(args::at(&array, r, *c));
            }
        }
        args::array_result(rows, cols, values)
    }
}

fn sort_key(a: &Scalar, b: &Scalar) -> std::cmp::Ordering {
    match coerce::compare(a, b) {
        Ok(Cmp::Lt) => std::cmp::Ordering::Less,
        Ok(Cmp::Eq) => std::cmp::Ordering::Equal,
        Ok(Cmp::Gt) => std::cmp::Ordering::Greater,
        Err(_) => std::cmp::Ordering::Equal,
    }
}

fn sort_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let array = match args
        .first()
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::arg_array(ctx, a))
    {
        Ok(a) => a,
        Err(e) => return err(e),
    };
    let sort_index = match args::opt_number(ctx, args, 1, 1.0).and_then(args::trunc_i64) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    if sort_index < 1 {
        return err(ErrorKind::Value);
    }
    let order = match args::opt_number(ctx, args, 2, 1.0).and_then(args::trunc_i64) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    if order != 1 && order != -1 {
        return err(ErrorKind::Value);
    }
    let by_col = match args::opt_bool(ctx, args, 3, false) {
        Ok(b) => b,
        Err(e) => return err(e),
    };
    let by_col = if array.rows == 1 && array.cols > 1 {
        true
    } else if array.cols == 1 && array.rows > 1 {
        false
    } else {
        by_col
    };
    let (n, breadth) = if by_col {
        (array.cols as usize, array.rows)
    } else {
        (array.rows as usize, array.cols)
    };
    if sort_index as u32 > breadth {
        return err(ErrorKind::Value);
    };
    let key_i = (sort_index as u32) - 1;
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| {
        let ka = if by_col {
            args::at(&array, key_i, a as u32)
        } else {
            args::at(&array, a as u32, key_i)
        };
        let kb = if by_col {
            args::at(&array, key_i, b as u32)
        } else {
            args::at(&array, b as u32, key_i)
        };
        let ord = sort_key(&ka, &kb);
        if order == -1 { ord.reverse() } else { ord }
    });
    reorder(&array, &idx, by_col)
}

fn reorder(array: &RuntimeArray, idx: &[usize], by_col: bool) -> RuntimeValue {
    if by_col {
        let cols = idx.len() as u32;
        let Ok(len) = args::check_shape(array.rows, cols) else {
            return err(ErrorKind::Num);
        };
        let mut values = Vec::with_capacity(len);
        for r in 0..array.rows {
            for &c in idx {
                values.push(args::at(array, r, c as u32));
            }
        }
        args::array_result(array.rows, cols, values)
    } else {
        let rows = idx.len() as u32;
        let Ok(len) = args::check_shape(rows, array.cols) else {
            return err(ErrorKind::Num);
        };
        let mut values = Vec::with_capacity(len);
        for &r in idx {
            for c in 0..array.cols {
                values.push(args::at(array, r as u32, c));
            }
        }
        args::array_result(rows, array.cols, values)
    }
}

fn sortby_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let array = match args
        .first()
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::arg_array(ctx, a))
    {
        Ok(a) => a,
        Err(e) => return err(e),
    };
    if args.len() < 2 {
        return err(ErrorKind::Value);
    }
    let mut keys: Vec<(RuntimeArray, i64)> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        if args[i].omitted {
            i += 1;
            continue;
        }
        let by = match args::arg_array(ctx, &args[i]) {
            Ok(a) => a,
            Err(e) => return err(e),
        };
        i += 1;
        let order = if i < args.len() && !args[i].omitted {
            match args::number(ctx, &args[i]).and_then(args::trunc_i64) {
                Ok(n) => {
                    i += 1;
                    n
                }
                Err(e) => return err(e),
            }
        } else {
            if i < args.len() && args[i].omitted {
                i += 1;
            }
            1
        };
        if order != 1 && order != -1 {
            return err(ErrorKind::Value);
        }
        keys.push((by, order));
    }
    if keys.is_empty() {
        return err(ErrorKind::Value);
    }
    let by_col = array.rows == 1 && array.cols > 1;
    let n = if by_col {
        array.cols as usize
    } else {
        array.rows as usize
    };
    for (by, _) in &keys {
        let klen = if by.cols == 1 { by.rows } else { by.cols } as usize;
        if klen != n {
            return err(ErrorKind::Value);
        }
    }
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| {
        for (by, order) in &keys {
            let ka = key_at(by, a);
            let kb = key_at(by, b);
            let ord = sort_key(&ka, &kb);
            let ord = if *order == -1 { ord.reverse() } else { ord };
            if ord != std::cmp::Ordering::Equal {
                return ord;
            }
        }
        std::cmp::Ordering::Equal
    });
    reorder(&array, &idx, by_col)
}

fn key_at(by: &RuntimeArray, i: usize) -> Scalar {
    if by.cols == 1 {
        args::at(by, i as u32, 0)
    } else {
        args::at(by, 0, i as u32)
    }
}

fn unique_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let array = match args
        .first()
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::arg_array(ctx, a))
    {
        Ok(a) => a,
        Err(e) => return err(e),
    };
    let by_col = match args::opt_bool(ctx, args, 1, false) {
        Ok(b) => {
            if array.rows == 1 && array.cols > 1 {
                true
            } else if array.cols == 1 && array.rows > 1 {
                false
            } else {
                b
            }
        }
        Err(e) => return err(e),
    };
    let exactly_once = match args::opt_bool(ctx, args, 2, false) {
        Ok(b) => b,
        Err(e) => return err(e),
    };
    let n = if by_col {
        array.cols as usize
    } else {
        array.rows as usize
    };
    let breadth = if by_col { array.rows } else { array.cols };
    let first_of = if breadth == 1 {
        unique_scalar_records(&array, n, by_col)
    } else {
        unique_wide_records(&array, n, by_col)
    };
    let kept: Vec<usize> = first_of
        .into_iter()
        .filter(|(_, count)| !exactly_once || *count == 1)
        .map(|(index, _)| index)
        .collect();
    if kept.is_empty() {
        return args::empty_array();
    }
    reorder(&array, &kept, by_col)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ScalarKey {
    Blank,
    Number(u64),
    Bool(bool),
    Text(String),
    Error(ErrorKind),
    Nan(usize),
}

fn scalar_key(value: Scalar, nonce: usize) -> ScalarKey {
    match value {
        Scalar::Empty => ScalarKey::Blank,
        Scalar::Number(number) if number.is_nan() => ScalarKey::Nan(nonce),
        Scalar::Number(number) => {
            ScalarKey::Number(if number == 0.0 { 0 } else { number.to_bits() })
        }
        Scalar::Bool(value) => ScalarKey::Bool(value),
        Scalar::Text(value) if value.is_empty() => ScalarKey::Blank,
        Scalar::Text(value) => ScalarKey::Text(value.to_ascii_lowercase()),
        Scalar::Error(value) => ScalarKey::Error(value),
    }
}

fn unique_scalar_records(array: &RuntimeArray, n: usize, by_col: bool) -> Vec<(usize, usize)> {
    let mut seen: HashMap<ScalarKey, usize> = HashMap::with_capacity(n);
    let mut records: Vec<(usize, usize)> = Vec::with_capacity(n);
    for i in 0..n {
        let value = if by_col {
            args::at(array, 0, i as u32)
        } else {
            args::at(array, i as u32, 0)
        };
        let key = scalar_key(value, i);
        if let Some(&entry) = seen.get(&key) {
            records[entry].1 += 1;
        } else {
            seen.insert(key, records.len());
            records.push((i, 1));
        }
    }
    records
}

fn unique_wide_records(array: &RuntimeArray, n: usize, by_col: bool) -> Vec<(usize, usize)> {
    let mut seen: HashMap<Vec<ScalarKey>, usize> = HashMap::with_capacity(n);
    let mut records: Vec<(usize, usize)> = Vec::with_capacity(n);
    for i in 0..n {
        let breadth = if by_col { array.rows } else { array.cols };
        let key: Vec<ScalarKey> = (0..breadth)
            .map(|j| {
                let value = if by_col {
                    args::at(array, j, i as u32)
                } else {
                    args::at(array, i as u32, j)
                };
                scalar_key(value, i)
            })
            .collect();
        if let Some(&entry) = seen.get(&key) {
            records[entry].1 += 1;
        } else {
            seen.insert(key, records.len());
            records.push((i, 1));
        }
    }
    records
}

fn sequence_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let rows = match args.first() {
        Some(a) => match args::number(ctx, a) {
            Ok(n) => match sequence_dimension(n) {
                Ok(v) => v,
                Err(e) => return err(e),
            },
            Err(e) => return err(e),
        },
        None => return err(ErrorKind::Value),
    };
    let cols = match args::opt_number(ctx, args, 1, 1.0) {
        Ok(n) => match sequence_dimension(n) {
            Ok(v) => v,
            Err(e) => return err(e),
        },
        Err(e) => return err(e),
    };
    let start = match args::opt_number(ctx, args, 2, 1.0) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let step = match args::opt_number(ctx, args, 3, 1.0) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let Ok(len) = args::check_shape(rows, cols) else {
        return err(ErrorKind::Num);
    };
    let mut values = Vec::with_capacity(len);
    for i in 0..len {
        values.push(Scalar::Number(start + step * (i as f64)));
    }
    args::array_result(rows, cols, values)
}

fn sequence_dimension(number: f64) -> Result<u32, ErrorKind> {
    if number < 0.0 {
        return Err(ErrorKind::Num);
    }
    let value = args::trunc_i64(number)?;
    if value == 0 {
        return Err(ErrorKind::Calc);
    }
    u32::try_from(value).map_err(|_| ErrorKind::Num)
}

fn randarray_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let rows = match args::opt_number(ctx, args, 0, 1.0) {
        Ok(n) => match args::pos_u32(n) {
            Ok(v) => v,
            Err(_) => return err(ErrorKind::Num),
        },
        Err(e) => return err(e),
    };
    let cols = match args::opt_number(ctx, args, 1, 1.0) {
        Ok(n) => match args::pos_u32(n) {
            Ok(v) => v,
            Err(_) => return err(ErrorKind::Num),
        },
        Err(e) => return err(e),
    };
    let min = match args::opt_number(ctx, args, 2, 0.0) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let max = match args::opt_number(ctx, args, 3, 1.0) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let integer = match args::opt_bool(ctx, args, 4, false) {
        Ok(b) => b,
        Err(e) => return err(e),
    };
    if min > max {
        return err(ErrorKind::Num);
    }
    let Ok(len) = args::check_shape(rows, cols) else {
        return err(ErrorKind::Num);
    };
    let mut values = Vec::with_capacity(len);
    for i in 0..len {
        let u = ctx.random_unit("RANDARRAY", i as u32);
        let v = if integer {
            // inclusive integer range
            let lo = min.ceil();
            let hi = max.floor();
            if lo > hi {
                return err(ErrorKind::Num);
            }
            let span = hi - lo + 1.0;
            (lo + (u * span).floor()).min(hi)
        } else {
            min + u * (max - min)
        };
        values.push(Scalar::Number(v));
    }
    args::array_result(rows, cols, values)
}

fn take_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    take_drop(ctx, args, false)
}

fn drop_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    take_drop(ctx, args, true)
}

fn take_drop(ctx: &mut EvalCtx<'_>, args: &[ArgVal], drop: bool) -> RuntimeValue {
    let array = match args
        .first()
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::arg_array(ctx, a))
    {
        Ok(a) => a,
        Err(e) => return err(e),
    };
    let rows_n = match args::opt_scalar(ctx, args, 1) {
        Ok(None) => None,
        Ok(Some(s)) => match coerce::to_number(&s).and_then(args::trunc_i64) {
            Ok(v) => Some(v),
            Err(e) => return err(e),
        },
        Err(e) => return err(e),
    };
    let cols_n = match args::opt_scalar(ctx, args, 2) {
        Ok(None) => None,
        Ok(Some(s)) => match coerce::to_number(&s).and_then(args::trunc_i64) {
            Ok(v) => Some(v),
            Err(e) => return err(e),
        },
        Err(e) => return err(e),
    };
    if rows_n.is_none() && cols_n.is_none() {
        return err(ErrorKind::Value);
    }
    // A vector's first size argument applies along its long axis (Excel TAKE/DROP).
    let (rows_n, cols_n) = if array.rows == 1 && array.cols > 1 && cols_n.is_none() {
        (None, rows_n)
    } else if array.cols == 1 && array.rows > 1 && cols_n.is_none() {
        (rows_n, None)
    } else {
        (rows_n, cols_n)
    };
    let (r0, r1) = match slice_range(array.rows, rows_n, drop) {
        Ok(v) => v,
        Err(e) => {
            return if e == ErrorKind::Calc {
                args::empty_array()
            } else {
                err(e)
            };
        }
    };
    let (c0, c1) = match slice_range(array.cols, cols_n, drop) {
        Ok(v) => v,
        Err(e) => {
            return if e == ErrorKind::Calc {
                args::empty_array()
            } else {
                err(e)
            };
        }
    };
    if r1 <= r0 || c1 <= c0 {
        return args::empty_array();
    }
    let rows = r1 - r0;
    let cols = c1 - c0;
    let Ok(len) = args::check_shape(rows, cols) else {
        return err(ErrorKind::Num);
    };
    let mut values = Vec::with_capacity(len);
    for r in r0..r1 {
        for c in c0..c1 {
            values.push(args::at(&array, r, c));
        }
    }
    args::array_result(rows, cols, values)
}

fn slice_range(n: u32, count: Option<i64>, drop: bool) -> Result<(u32, u32), ErrorKind> {
    let Some(count) = count else {
        return Ok((0, n));
    };
    if count == 0 {
        return if drop {
            Ok((0, n))
        } else {
            Err(ErrorKind::Calc)
        };
    }
    let n_i = i64::from(n);
    if drop {
        if count > 0 {
            if count >= n_i {
                return Err(ErrorKind::Calc);
            }
            Ok((count as u32, n))
        } else {
            let k = count.unsigned_abs();
            if k >= u64::from(n) {
                return Err(ErrorKind::Calc);
            }
            Ok((0, n - k as u32))
        }
    } else if count > 0 {
        let take = count.min(n_i) as u32;
        Ok((0, take))
    } else {
        let k = count.unsigned_abs().min(u64::from(n)) as u32;
        Ok((n - k, n))
    }
}

fn chooserows_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    choose_axis(ctx, args, true)
}

fn choosecols_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    choose_axis(ctx, args, false)
}

fn choose_axis(ctx: &mut EvalCtx<'_>, args: &[ArgVal], rows: bool) -> RuntimeValue {
    let array = match args
        .first()
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::arg_array(ctx, a))
    {
        Ok(a) => a,
        Err(e) => return err(e),
    };
    let limit = if rows { array.rows } else { array.cols };
    let mut idxs: Vec<u32> = Vec::new();
    for arg in args.iter().skip(1) {
        if arg.omitted {
            continue;
        }
        match collect_indices(ctx, arg, limit, &mut idxs) {
            Ok(()) => {}
            Err(e) => return err(e),
        }
    }
    if idxs.is_empty() {
        return err(ErrorKind::Value);
    }
    reorder(
        &array,
        &idxs.iter().map(|&i| i as usize).collect::<Vec<_>>(),
        !rows,
    )
}

fn collect_indices(
    ctx: &mut EvalCtx<'_>,
    arg: &ArgVal,
    limit: u32,
    out: &mut Vec<u32>,
) -> Result<(), ErrorKind> {
    let array = args::arg_array(ctx, arg)?;
    for s in array.values.iter() {
        if let Some(e) = s.error() {
            return Err(e);
        }
        let n = args::trunc_i64(coerce::to_number(s)?)?;
        if n == 0 {
            return Err(ErrorKind::Value);
        }
        let idx = if n > 0 {
            if n as u32 > limit {
                return Err(ErrorKind::Value);
            }
            (n as u32) - 1
        } else {
            let k = n.unsigned_abs();
            if k > u64::from(limit) {
                return Err(ErrorKind::Value);
            }
            limit - k as u32
        };
        out.push(idx);
    }
    Ok(())
}

fn vstack_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    stack(ctx, args, true)
}

fn hstack_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    stack(ctx, args, false)
}

fn stack(ctx: &mut EvalCtx<'_>, args: &[ArgVal], vertical: bool) -> RuntimeValue {
    let mut parts: Vec<RuntimeArray> = Vec::new();
    for a in args {
        if a.omitted {
            continue;
        }
        if let Some(e) = a.value.error_kind() {
            return err(e);
        }
        match args::arg_array(ctx, a) {
            Ok(p) => parts.push(p),
            Err(e) => return err(e),
        }
    }
    if parts.is_empty() {
        return err(ErrorKind::Value);
    }
    if vertical {
        let cols = parts.iter().map(|p| p.cols).max().unwrap_or(1);
        let rows: u32 = parts
            .iter()
            .map(|p| p.rows)
            .fold(0u32, |a, b| a.saturating_add(b));
        let Ok(len) = args::check_shape(rows, cols) else {
            return err(ErrorKind::Num);
        };
        let mut values = Vec::with_capacity(len);
        for p in &parts {
            for r in 0..p.rows {
                for c in 0..cols {
                    if c < p.cols {
                        values.push(args::at(p, r, c));
                    } else {
                        values.push(Scalar::Error(ErrorKind::Na));
                    }
                }
            }
        }
        args::array_result(rows, cols, values)
    } else {
        let rows = parts.iter().map(|p| p.rows).max().unwrap_or(1);
        let cols: u32 = parts
            .iter()
            .map(|p| p.cols)
            .fold(0u32, |a, b| a.saturating_add(b));
        let Ok(len) = args::check_shape(rows, cols) else {
            return err(ErrorKind::Num);
        };
        let mut values = Vec::with_capacity(len);
        for r in 0..rows {
            for p in &parts {
                for c in 0..p.cols {
                    if r < p.rows {
                        values.push(args::at(p, r, c));
                    } else {
                        values.push(Scalar::Error(ErrorKind::Na));
                    }
                }
            }
        }
        args::array_result(rows, cols, values)
    }
}

fn tocol_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    flatten(ctx, args, true)
}

fn torow_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    flatten(ctx, args, false)
}

fn flatten(ctx: &mut EvalCtx<'_>, args: &[ArgVal], to_col: bool) -> RuntimeValue {
    let array = match args.first().ok_or(ErrorKind::Value).and_then(|a| {
        if let Some(e) = a.value.error_kind() {
            return Err(e);
        }
        args::arg_array(ctx, a)
    }) {
        Ok(a) => a,
        Err(e) => return err(e),
    };
    let ignore = match args::opt_number(ctx, args, 1, 0.0).and_then(args::trunc_i64) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    if !(0..=3).contains(&ignore) {
        return err(ErrorKind::Value);
    }
    let by_col = match args::opt_bool(ctx, args, 2, false) {
        Ok(b) => b,
        Err(e) => return err(e),
    };
    let skip_blanks = ignore == 1 || ignore == 3;
    let skip_err = ignore == 2 || ignore == 3;
    let count = array
        .values
        .iter()
        .filter(|value| !skip_blanks || !args::is_blank(value))
        .filter(|value| !skip_err || value.error().is_none())
        .count();
    if count == 0 {
        return args::empty_array();
    }
    let Ok(n) = u32::try_from(count) else {
        return err(ErrorKind::Num);
    };
    let shape = if to_col { (n, 1) } else { (1, n) };
    let Ok(len) = args::check_shape(shape.0, shape.1) else {
        return err(ErrorKind::Num);
    };
    let mut values = Vec::with_capacity(len);
    if by_col {
        for c in 0..array.cols {
            for r in 0..array.rows {
                push_flat(&array, r, c, skip_blanks, skip_err, &mut values);
            }
        }
    } else {
        for r in 0..array.rows {
            for c in 0..array.cols {
                push_flat(&array, r, c, skip_blanks, skip_err, &mut values);
            }
        }
    }
    args::array_result(shape.0, shape.1, values)
}

fn push_flat(
    array: &RuntimeArray,
    r: u32,
    c: u32,
    skip_blanks: bool,
    skip_err: bool,
    out: &mut Vec<Scalar>,
) {
    let s = args::at(array, r, c);
    if skip_blanks && args::is_blank(&s) {
        return;
    }
    if skip_err && s.error().is_some() {
        return;
    }
    out.push(s);
}

fn wraprows_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    wrap(ctx, args, true)
}

fn wrapcols_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    wrap(ctx, args, false)
}

fn wrap(ctx: &mut EvalCtx<'_>, args: &[ArgVal], as_rows: bool) -> RuntimeValue {
    let array = match args.first().ok_or(ErrorKind::Value).and_then(|a| {
        if let Some(e) = a.value.error_kind() {
            return Err(e);
        }
        args::arg_array(ctx, a)
    }) {
        Ok(a) => a,
        Err(e) => return err(e),
    };
    let wrap_count = match args
        .get(1)
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::number(ctx, a))
    {
        Ok(n) => match args::pos_u32(n) {
            Ok(v) => v,
            Err(_) => return err(ErrorKind::Num),
        },
        Err(e) => return err(e),
    };
    if as_rows {
        if wrap_count > u32::from(MAX_COLS) {
            return err(ErrorKind::Num);
        }
    } else if wrap_count > MAX_ROWS {
        return err(ErrorKind::Num);
    }
    let pad = match args::opt_scalar(ctx, args, 2) {
        Ok(Some(s)) => s,
        Ok(None) => Scalar::Error(ErrorKind::Na),
        Err(e) => return err(e),
    };
    let vec: Vec<Scalar> = array.values.iter().cloned().collect();
    let n = vec.len() as u32;
    let groups = n.div_ceil(wrap_count);
    if as_rows {
        let rows = groups;
        let cols = wrap_count;
        let Ok(len) = args::check_shape(rows, cols) else {
            return err(ErrorKind::Num);
        };
        let mut values = Vec::with_capacity(len);
        for i in 0..len {
            values.push(vec.get(i).cloned().unwrap_or_else(|| pad.clone()));
        }
        args::array_result(rows, cols, values)
    } else {
        let rows = wrap_count;
        let cols = groups;
        let Ok(len) = args::check_shape(rows, cols) else {
            return err(ErrorKind::Num);
        };
        let mut values = Vec::with_capacity(len);
        for r in 0..rows {
            for c in 0..cols {
                let i = (c * rows + r) as usize;
                values.push(vec.get(i).cloned().unwrap_or_else(|| pad.clone()));
            }
        }
        args::array_result(rows, cols, values)
    }
}

fn expand_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let array = match args.first().ok_or(ErrorKind::Value).and_then(|a| {
        if let Some(e) = a.value.error_kind() {
            return Err(e);
        }
        args::arg_array(ctx, a)
    }) {
        Ok(a) => a,
        Err(e) => return err(e),
    };
    let rows = match args.get(1) {
        Some(a) if !a.omitted => match args::number(ctx, a).and_then(args::trunc_i64) {
            Ok(n) if n < 1 => return err(ErrorKind::Value),
            Ok(n) => match u32::try_from(n) {
                Ok(v) => v,
                Err(_) => return err(ErrorKind::Num),
            },
            Err(e) => return err(e),
        },
        _ => array.rows,
    };
    let cols = match args.get(2) {
        Some(a) if !a.omitted => match args::number(ctx, a).and_then(args::trunc_i64) {
            Ok(n) if n < 1 => return err(ErrorKind::Value),
            Ok(n) => match u32::try_from(n) {
                Ok(v) => v,
                Err(_) => return err(ErrorKind::Num),
            },
            Err(e) => return err(e),
        },
        _ => array.cols,
    };
    if rows < array.rows || cols < array.cols {
        return err(ErrorKind::Value);
    }
    let pad = match args::opt_scalar(ctx, args, 3) {
        Ok(Some(s)) => s,
        Ok(None) => Scalar::Error(ErrorKind::Na),
        Err(e) => return err(e),
    };
    let Ok(len) = args::check_shape(rows, cols) else {
        return err(ErrorKind::Num);
    };
    let mut values = Vec::with_capacity(len);
    for r in 0..rows {
        for c in 0..cols {
            if r < array.rows && c < array.cols {
                values.push(args::at(&array, r, c));
            } else {
                values.push(pad.clone());
            }
        }
    }
    args::array_result(rows, cols, values)
}
