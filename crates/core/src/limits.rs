//! Grid and formula size limits (Excel’s grid, spec F-1.2).

/// Maximum number of rows in a sheet (`1..=1_048_576` in A1).
pub const MAX_ROWS: u32 = 1_048_576;

/// Maximum number of columns in a sheet (`A..=XFD` in A1).
pub const MAX_COLS: u16 = 16_384;

/// Maximum formula source length in UTF-8 bytes.
pub const MAX_FORMULA_LEN: usize = 8_192;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::addr::{col_from_letters, col_to_letters};

    #[test]
    fn limits_match_excel_grid() {
        assert_eq!(MAX_ROWS, 1_048_576);
        assert_eq!(MAX_COLS, 16_384);
        assert_eq!(MAX_FORMULA_LEN, 8_192);
        assert_eq!(col_from_letters("XFD").unwrap(), MAX_COLS - 1);
        assert_eq!(col_to_letters(MAX_COLS - 1).unwrap(), "XFD");
    }
}
