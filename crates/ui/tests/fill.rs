//! Fill-series edge cases.

use omacell_ui::{FillKind, detect_series, extend_series};

#[test]
fn constants_are_copy_and_empty_extension_is_total() {
    assert_eq!(detect_series(&[4.0, 4.0]), FillKind::Copy);
    assert_eq!(extend_series(&[], FillKind::Copy, 3), vec![0.0; 3]);
    assert_eq!(
        extend_series(&[4.0, 4.0], FillKind::Linear, 2),
        vec![4.0, 4.0]
    );
}
