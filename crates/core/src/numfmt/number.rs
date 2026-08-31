//! Decimal helpers: 15-digit display rounding and half-away-from-zero.

/// Excel F-2.6: round to 15 significant digits before display.
#[must_use]
pub fn excel_precision_15(n: f64) -> f64 {
    if !n.is_finite() || n == 0.0 {
        return n;
    }
    let sign = if n < 0.0 { -1.0 } else { 1.0 };
    let a = n.abs();
    let log = a.log10();
    if !log.is_finite() {
        return n;
    }
    let exp = log.floor();
    let places = 14.0 - exp;
    if places > 20.0 {
        return n;
    }
    if places < -15.0 {
        return n;
    }
    round_half_away(a, places as i32) * sign
}

/// Round `n` to `places` decimal places, half away from zero.
#[must_use]
pub fn round_half_away(n: f64, places: i32) -> f64 {
    if !n.is_finite() {
        return n;
    }
    if places > 20 {
        return n;
    }
    if places < -20 {
        return 0.0;
    }
    let p = 10f64.powi(places);
    let x = n * p;
    if !x.is_finite() {
        return n;
    }
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let ax = x.abs();
    let floor = ax.floor();
    let r = if ax - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    };
    sign * r / p
}

/// 15-digit mantissa and scientific exponent.
#[must_use]
pub fn sig15(n: f64) -> (u64, i32) {
    let a = excel_precision_15(n.abs());
    if a == 0.0 || !a.is_finite() {
        return (0, 0);
    }
    let log = a.log10();
    let mut exp = log.floor() as i32;
    let scaled = a / 10f64.powi(exp) * 1e14;
    if !scaled.is_finite() {
        return (0, exp);
    }
    let mut m = round_half_away(scaled, 0);
    if m >= 1e15 {
        m /= 10.0;
        exp += 1;
    }
    if m < 1e14 && exp > i32::MIN {
        m *= 10.0;
        exp -= 1;
    }
    (m.max(0.0) as u64, exp)
}

/// Split a rounded non-negative number into integer and fractional digit vectors.
pub fn split_fixed(n: f64, frac_places: usize) -> (Vec<u8>, Vec<u8>) {
    let a = n.abs();
    if a == 0.0 || !a.is_finite() {
        return (vec![0], vec![0; frac_places]);
    }
    let rounded = round_half_away(a, frac_places.min(18) as i32);
    if rounded == 0.0 {
        return (vec![0], vec![0; frac_places]);
    }
    let (mant, exp) = sig15(rounded);
    if mant == 0 {
        return (vec![0], vec![0; frac_places]);
    }
    let mut digits = mant.to_string().into_bytes();
    while digits.len() < 15 {
        digits.insert(0, b'0');
    }
    let mut int_digits: Vec<u8> = Vec::new();
    let mut frac_digits: Vec<u8> = vec![0; frac_places];
    if exp >= 0 {
        let int_len = exp as usize + 1;
        if int_len <= digits.len() {
            int_digits.extend(digits[..int_len].iter().map(|d| d - b'0'));
            for (i, d) in digits[int_len..].iter().take(frac_places).enumerate() {
                frac_digits[i] = d - b'0';
            }
        } else {
            int_digits.extend(digits.iter().map(|d| d - b'0'));
            int_digits.resize(int_len, 0);
        }
    } else {
        int_digits.push(0);
        let lead_zeros = (-exp - 1) as usize;
        for (i, slot) in frac_digits.iter_mut().enumerate() {
            if i >= lead_zeros {
                let di = i - lead_zeros;
                *slot = if di < digits.len() {
                    digits[di] - b'0'
                } else {
                    0
                };
            }
        }
    }
    while int_digits.len() > 1 && int_digits[0] == 0 {
        int_digits.remove(0);
    }
    if int_digits.is_empty() {
        int_digits.push(0);
    }
    (int_digits, frac_digits)
}

/// Group an integer digit string every 3 from the right.
pub fn group_int(digits: &[u8], thousands: char) -> String {
    let mut s = String::new();
    for (i, d) in digits.iter().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            s.push(thousands);
        }
        s.push(char::from(b'0' + *d));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_away() {
        assert_eq!(round_half_away(1.25, 1), 1.3);
        assert_eq!(round_half_away(-1.25, 1), -1.3);
        assert_eq!(round_half_away(2.5, 0), 3.0);
    }
}
