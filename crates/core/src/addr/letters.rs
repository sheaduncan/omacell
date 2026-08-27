//! Column letters ↔ 0-based index up to `XFD`.

use crate::error::CoreError;
use crate::limits::MAX_COLS;

/// Convert a 0-based column index to Excel letters (`0` → `A`, `16383` → `XFD`).
///
/// ```
/// use omacell_core::addr::col_to_letters;
/// assert_eq!(col_to_letters(0).unwrap(), "A");
/// assert_eq!(col_to_letters(25).unwrap(), "Z");
/// assert_eq!(col_to_letters(26).unwrap(), "AA");
/// assert_eq!(col_to_letters(16_383).unwrap(), "XFD");
/// ```
pub fn col_to_letters(col: u16) -> Result<String, CoreError> {
    if u32::from(col) >= u32::from(MAX_COLS) {
        return Err(CoreError::addr_ref(format!(
            "column index {col} is out of range"
        )));
    }
    let mut n = u32::from(col) + 1;
    let mut tmp = [0u8; 3];
    let mut len = 0usize;
    while n > 0 {
        n -= 1;
        tmp[len] = b'A' + (n % 26) as u8;
        len += 1;
        n /= 26;
    }
    let mut out = String::with_capacity(len);
    for i in (0..len).rev() {
        out.push(char::from(tmp[i]));
    }
    Ok(out)
}

/// Parse Excel column letters (case-insensitive) to a 0-based index.
///
/// Out-of-range letters (`XFE`, `AAAA`) return a `#REF!`-class error.
///
/// ```
/// use omacell_core::addr::col_from_letters;
/// assert_eq!(col_from_letters("A").unwrap(), 0);
/// assert_eq!(col_from_letters("xfd").unwrap(), 16_383);
/// assert!(col_from_letters("XFE").is_err());
/// ```
pub fn col_from_letters(letters: &str) -> Result<u16, CoreError> {
    if letters.is_empty() {
        return Err(CoreError::addr_parse("column letters must not be empty"));
    }
    let mut n: u32 = 0;
    for c in letters.chars() {
        if !c.is_ascii_alphabetic() {
            return Err(CoreError::addr_parse(format!(
                "invalid column letter {c:?}"
            )));
        }
        let d = u32::from(c.to_ascii_uppercase()) - u32::from(b'A') + 1;
        n = n.saturating_mul(26).saturating_add(d);
        if n > u32::from(MAX_COLS) {
            return Err(CoreError::addr_ref(format!(
                "column {letters} is beyond XFD"
            )));
        }
    }
    Ok((n - 1) as u16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::MAX_COLS;

    #[test]
    fn letters_roundtrip_every_column() {
        for col in 0..MAX_COLS {
            let letters = col_to_letters(col).unwrap();
            assert_eq!(col_from_letters(&letters).unwrap(), col, "{letters}");
            assert_eq!(
                col_from_letters(&letters.to_ascii_lowercase()).unwrap(),
                col
            );
        }
    }

    #[test]
    fn known_columns() {
        assert_eq!(col_from_letters("A").unwrap(), 0);
        assert_eq!(col_from_letters("Z").unwrap(), 25);
        assert_eq!(col_from_letters("AA").unwrap(), 26);
        assert_eq!(col_from_letters("AZ").unwrap(), 51);
        assert_eq!(col_from_letters("BA").unwrap(), 52);
        assert_eq!(col_from_letters("XFD").unwrap(), MAX_COLS - 1);
        assert!(
            col_from_letters("XFE").unwrap_err().excel_error()
                == Some(crate::error::ErrorKind::Ref)
        );
        assert!(col_from_letters("").is_err());
        assert!(col_from_letters("A1").is_err());
    }
}
