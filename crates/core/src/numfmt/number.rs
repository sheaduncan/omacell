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
    let (mant, exp) = sig15(a);
    if mant == 0 {
        return (vec![0], vec![0; frac_places]);
    }
    let mut significant = mant.to_string().into_bytes();
    while significant.len() < 15 {
        significant.insert(0, b'0');
    }

    // `mant` is Excel's 15-digit decimal coefficient. Round that decimal,
    // rather than the original binary float, at the requested display place.
    let keep = i64::from(exp)
        .saturating_add(1)
        .saturating_add(i64::try_from(frac_places).unwrap_or(i64::MAX));
    let mut scaled = if keep < 0 {
        vec![b'0']
    } else if keep == 0 {
        if significant[0] >= b'5' {
            vec![b'1']
        } else {
            vec![b'0']
        }
    } else {
        let keep = match usize::try_from(keep) {
            Ok(keep) => keep,
            Err(_) => return (vec![0], vec![0; frac_places]),
        };
        if keep >= significant.len() {
            significant.resize(keep, b'0');
            significant
        } else {
            let round_up = significant[keep] >= b'5';
            significant.truncate(keep);
            if round_up {
                increment_decimal(&mut significant);
            }
            significant
        }
    };

    if frac_places == 0 {
        return (
            scaled.into_iter().map(|digit| digit - b'0').collect(),
            Vec::new(),
        );
    }

    if scaled.len() <= frac_places {
        let mut fractional = vec![0; frac_places - scaled.len()];
        fractional.extend(scaled.into_iter().map(|digit| digit - b'0'));
        return (vec![0], fractional);
    }

    let fractional = scaled.split_off(scaled.len() - frac_places);
    let integer = scaled.into_iter().map(|digit| digit - b'0').collect();
    let fractional = fractional.into_iter().map(|digit| digit - b'0').collect();
    (integer, fractional)
}

fn increment_decimal(digits: &mut Vec<u8>) {
    for digit in digits.iter_mut().rev() {
        if *digit < b'9' {
            *digit += 1;
            return;
        }
        *digit = b'0';
    }
    digits.insert(0, b'1');
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
