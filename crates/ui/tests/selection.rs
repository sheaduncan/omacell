//! Selection property tests.

use omacell_core::addr::CellRef;
use omacell_core::addr::SheetId;
use omacell_ui::{Area, ExtendMode, Selection, SelectionStats, SelectionStatsProvider};
use proptest::prelude::*;

proptest! {
    #[test]
    fn move_stays_in_grid(drow in -8i64..8, dcol in -8i64..8, n in 1u32..5) {
        let mut sel = Selection::a1(SheetId::new(0));
        for _ in 0..n {
            sel.move_by(drow, dcol);
        }
        assert!(sel.cursor.row < 1_048_576);
        assert!(sel.cursor.col < 16_384);
        assert!(!sel.areas.is_empty());
        assert!(sel.cell_count() >= 1);
    }

    #[test]
    fn extend_grows_an_area(steps in 1u32..10) {
        let mut sel = Selection::a1(SheetId::new(0));
        sel.extend = ExtendMode::Extend;
        for _ in 0..steps {
            sel.move_by(1, 1);
        }
        assert_eq!(sel.areas.len(), 1);
        assert!(sel.active().cells() >= 1);
    }
}

#[test]
fn extreme_public_inputs_do_not_overflow() {
    let mut sel = Selection::a1(SheetId::new(0));
    sel.move_by(i64::MAX, i64::MAX);
    assert_eq!(sel.cursor.row, 1_048_575);
    assert_eq!(sel.cursor.col, 16_383);
    sel.move_by(i64::MIN, i64::MIN);
    assert_eq!((sel.cursor.row, sel.cursor.col), (0, 0));

    let start = CellRef {
        sheet: None,
        row: 0,
        col: 0,
        row_abs: false,
        col_abs: false,
    };
    let end = CellRef {
        row: u32::MAX,
        col: u16::MAX,
        ..start
    };
    assert_eq!(
        Area { start, end }.cells(),
        (u64::from(u32::MAX) + 1) * 65_536
    );
}

#[test]
fn selection_statistics_are_supplied_without_coupling_to_a_frontend() {
    struct Stats;
    impl SelectionStatsProvider for Stats {
        fn stats(&self, selection: &Selection) -> SelectionStats {
            SelectionStats {
                cells: selection.cell_count(),
                numeric: 2,
                sum: Some(6.0),
                average: Some(3.0),
                min: Some(2.0),
                max: Some(4.0),
            }
        }
    }

    let selection = Selection::a1(SheetId::new(0));
    assert_eq!(selection.stats(&Stats).average, Some(3.0));
}
