//! Storage vs HashMap<(u32,u16), CellSlot> oracle (WP-02 acceptance).

use std::collections::HashMap;

use omacell_core::error::codes;
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use omacell_core::storage::{CellSlot, SheetStore};
use omacell_core::value::Value;
use proptest::prelude::*;
use proptest::test_runner::{Config as ProptestConfig, FileFailurePersistence};

const T_ROWS: u32 = 300;
const T_COLS: u16 = 24;

#[derive(Clone, Debug)]
enum Op {
    Set { row: u32, col: u16, n: i32 },
    Clear { row: u32, col: u16 },
    ShiftRows { at: u32, count: i8 },
    ShiftCols { at: u16, count: i8 },
}

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0u32..T_ROWS, 0u16..T_COLS, -100i32..100i32).prop_map(|(row, col, n)| Op::Set {
            row,
            col,
            n
        }),
        (0u32..T_ROWS, 0u16..T_COLS).prop_map(|(row, col)| Op::Clear { row, col }),
        (0u32..T_ROWS, -3i8..=3i8).prop_map(|(at, count)| Op::ShiftRows { at, count }),
        (0u16..T_COLS, -3i8..=3i8).prop_map(|(at, count)| Op::ShiftCols { at, count }),
    ]
}

fn oracle_shift_rows(
    map: &mut HashMap<(u32, u16), CellSlot>,
    at: u32,
    count: i32,
) -> Result<(), ()> {
    if count > 0 {
        let n = count as u32;
        for (r, _) in map.keys() {
            if *r >= at && r.checked_add(n).is_none_or(|nr| nr >= MAX_ROWS) {
                return Err(());
            }
        }
        let mut next = HashMap::new();
        for ((r, c), v) in map.drain() {
            let nr = if r >= at { r + n } else { r };
            next.insert((nr, c), v);
        }
        *map = next;
    } else if count < 0 {
        let n = (-count) as u32;
        let mut next = HashMap::new();
        for ((r, c), v) in map.drain() {
            if r < at {
                next.insert((r, c), v);
            } else if r >= at.saturating_add(n) {
                next.insert((r - n, c), v);
            }
        }
        *map = next;
    }
    Ok(())
}

fn oracle_shift_cols(
    map: &mut HashMap<(u32, u16), CellSlot>,
    at: u16,
    count: i32,
) -> Result<(), ()> {
    if count > 0 {
        let n = count as u16;
        for (_, c) in map.keys() {
            if *c >= at {
                match c.checked_add(n) {
                    Some(nc) if u32::from(nc) < u32::from(MAX_COLS) => {}
                    _ => return Err(()),
                }
            }
        }
        let mut next = HashMap::new();
        for ((r, c), v) in map.drain() {
            let nc = if c >= at { c + n } else { c };
            next.insert((r, nc), v);
        }
        *map = next;
    } else if count < 0 {
        let n = (-count) as u16;
        let mut next = HashMap::new();
        for ((r, c), v) in map.drain() {
            if c < at {
                next.insert((r, c), v);
            } else if c >= at.saturating_add(n) {
                next.insert((r, c - n), v);
            }
        }
        *map = next;
    }
    Ok(())
}

fn apply_store(store: &mut SheetStore, op: &Op) -> Result<(), ()> {
    match *op {
        Op::Set { row, col, n } => {
            store
                .set(row, col, CellSlot::number(f64::from(n)))
                .map_err(|_| ())?;
            Ok(())
        }
        Op::Clear { row, col } => {
            store.clear(row, col).map_err(|_| ())?;
            Ok(())
        }
        Op::ShiftRows { at, count } => {
            if count == 0 {
                return Ok(());
            }
            store.shift_rows(at, i32::from(count)).map_err(|e| {
                assert_eq!(e.code, codes::ADDR_REF);
            })
        }
        Op::ShiftCols { at, count } => {
            if count == 0 {
                return Ok(());
            }
            store.shift_cols(at, i32::from(count)).map_err(|e| {
                assert_eq!(e.code, codes::ADDR_REF);
            })
        }
    }
}

fn apply_oracle(map: &mut HashMap<(u32, u16), CellSlot>, op: &Op) -> Result<(), ()> {
    match *op {
        Op::Set { row, col, n } => {
            map.insert((row, col), CellSlot::number(f64::from(n)));
            Ok(())
        }
        Op::Clear { row, col } => {
            map.remove(&(row, col));
            Ok(())
        }
        Op::ShiftRows { at, count } => {
            if count == 0 {
                return Ok(());
            }
            oracle_shift_rows(map, at, i32::from(count))
        }
        Op::ShiftCols { at, count } => {
            if count == 0 {
                return Ok(());
            }
            oracle_shift_cols(map, at, i32::from(count))
        }
    }
}

fn assert_eq_store(store: &SheetStore, map: &HashMap<(u32, u16), CellSlot>) {
    assert_eq!(store.len() as usize, map.len());
    for (&(r, c), slot) in map {
        let got = store.get(r, c).unwrap();
        assert_eq!(got.copied(), Some(*slot), "mismatch at ({r},{c})");
    }
    for (r, c, slot) in store.iter() {
        assert_eq!(map.get(&(r, c)).copied(), Some(slot), "extra at ({r},{c})");
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        cases: 256,
        ..ProptestConfig::default()
    })]

    #[test]
    fn store_matches_hashmap_oracle(ops in prop::collection::vec(arb_op(), 0..48)) {
        let mut store = SheetStore::new();
        let mut map = HashMap::new();
        for op in &ops {
            let a = apply_store(&mut store, op);
            let b = apply_oracle(&mut map, op);
            prop_assert_eq!(a.is_ok(), b.is_ok(), "op {:?}", op);
            if a.is_err() {
                continue;
            }
        }
        assert_eq_store(&store, &map);
    }
}

#[test]
fn shift_across_block_boundary() {
    let mut store = SheetStore::new();
    let mut map = HashMap::new();
    store.set(250, 0, CellSlot::number(1.0)).unwrap();
    map.insert((250, 0), CellSlot::number(1.0));
    store.set(260, 0, CellSlot::number(2.0)).unwrap();
    map.insert((260, 0), CellSlot::number(2.0));
    store.shift_rows(256, 4).unwrap();
    oracle_shift_rows(&mut map, 256, 4).unwrap();
    assert_eq_store(&store, &map);
    assert_eq!(
        store.get(264, 0).unwrap().unwrap().value,
        Value::Number(2.0)
    );
}
