//! Frozen panes and hidden rows.

use omacell_ui::Viewport;

#[test]
fn hidden_rows_are_skipped_and_hit_test_still_works() {
    let mut vp = Viewport::default();
    vp.rows.set_hidden(0, true).unwrap();
    assert_eq!(vp.row_px(0), 0);
    assert_eq!(vp.first_data_row(), 1);
    let idx = vp.hit_row(0);
    assert_ne!(idx, u32::MAX);
}

#[test]
fn freeze_keeps_header_rows() {
    let mut vp = Viewport::default();
    vp.freeze.rows = 2;
    vp.first_row = 10;
    assert_eq!(vp.first_data_row(), 10);
    vp.ensure_row_visible(3);
    assert_eq!(vp.first_row, 3);
}

#[test]
fn zoom_clamps() {
    let mut vp = Viewport::default();
    vp.set_zoom(100.0);
    assert!(vp.zoom <= 8.0);
    vp.set_zoom(0.01);
    assert!(vp.zoom >= 0.25);
}
