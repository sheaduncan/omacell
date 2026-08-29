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
    vp.set_zoom(f64::NAN);
    assert!(vp.zoom.is_finite());
}

#[test]
fn ensure_visible_scrolls_forward_as_well_as_backward() {
    let mut vp = Viewport {
        height_px: 100,
        width_px: 128,
        ..Viewport::default()
    };
    vp.ensure_row_visible(20);
    assert!(vp.first_row > 0);
    vp.ensure_col_visible(10);
    assert!(vp.first_col > 0);
    assert_eq!(vp.hit_row(0), vp.first_row);
    assert_eq!(vp.hit_col(0), vp.first_col);
}

#[test]
fn page_and_screen_metrics_follow_viewport_geometry() {
    let mut viewport = Viewport {
        width_px: 256,
        height_px: 100,
        first_row: 10,
        first_col: 4,
        ..Viewport::default()
    };
    assert_eq!(viewport.page_rows(), 5);
    assert_eq!(viewport.page_cols(), 4);
    assert_eq!(viewport.screen_rows(), (10, 12, 14));

    viewport.zoom = 2.0;
    assert_eq!(viewport.page_rows(), 3);
    assert_eq!(viewport.page_cols(), 2);
}
