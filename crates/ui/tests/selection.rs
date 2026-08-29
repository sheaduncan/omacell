//! Selection property tests.

use omacell_core::addr::SheetId;
use omacell_ui::{ExtendMode, Selection};
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
