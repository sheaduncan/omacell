//! Excel `General` algorithm (F-2.3, F-2.6).

use crate::numfmt::number::{excel_precision_15, round_half_away, sig15};

/// Display `n` with Excel General (11 characters excluding sign).
#[must_use]
pub fn general(n: f64) -> String {
    general_for_width(n, 11)
}

/// Width-aware General: shorten, then scientific, then `#`s.
#[must_use]
pub fn general_for_width(n: f64, width: usize) -> String {
    if !n.is_finite() {
        return "#NUM!".to_string();
    }
    if n == 0.0 {
        return "0".to_string();
    }
    let neg = n < 0.0;
    let a = excel_precision_15(n.abs());
    if a == 0.0 {
        return "0".to_string();
    }
    let body = general_abs(a, width.max(1));
    if neg {
        format!("-{body}")
    } else {
        body
    }
}

fn general_abs(a: f64, budget: usize) -> String {
    let (mant, exp) = sig15(a);
    if a >= 1e11 || (a > 0.0 && a < 1e-4) {
        return general_sci(mant, exp, budget);
    }
    let int_digits = if exp >= 0 { exp as usize + 1 } else { 1 };
    if int_digits > budget {
        return general_sci(mant, exp, budget);
    }
    if integer_p(mant, exp) && exp >= 0 && int_digits <= budget {
        return int_string(mant, exp);
    }
    let frac_places = if exp >= 0 {
        budget.saturating_sub(int_digits).saturating_sub(1)
    } else {
        budget.saturating_sub(2)
    };
    let rounded = round_half_away(a, frac_places.min(20) as i32).max(0.0);
    if rounded == 0.0 {
        return "0".to_string();
    }
    if rounded >= 1e11 {
        let (m, e) = sig15(rounded);
        return general_sci(m, e, budget);
    }
    format_fixed(rounded, frac_places)
}

fn integer_p(mant: u64, exp: i32) -> bool {
    if exp < 0 {
        return false;
    }
    if exp >= 14 {
        return true;
    }
    let keep = exp as u32 + 1;
    mant % 10u64.pow(14 - keep) == 0
}

fn int_string(mant: u64, exp: i32) -> String {
    if exp < 0 {
        return "0".to_string();
    }
    let mut s = format!("{mant:015}");
    let int_len = exp as usize + 1;
    if int_len <= 15 {
        s.truncate(int_len);
    } else {
        s.push_str(&"0".repeat(int_len - 15));
    }
    s.trim_start_matches('0').to_string().if_empty_zero()
}

trait IfEmptyZero {
    fn if_empty_zero(self) -> String;
}
impl IfEmptyZero for String {
    fn if_empty_zero(self) -> String {
        if self.is_empty() { "0".into() } else { self }
    }
}

fn format_fixed(a: f64, frac_places: usize) -> String {
    let (mant, exp) = sig15(a);
    let digits = format!("{mant:015}");
    let mut int_part = String::new();
    let mut frac_part = String::new();
    if exp >= 0 {
        let int_len = exp as usize + 1;
        if int_len <= digits.len() {
            int_part.push_str(&digits[..int_len]);
            frac_part.push_str(&digits[int_len..]);
        } else {
            int_part.push_str(&digits);
            int_part.push_str(&"0".repeat(int_len - digits.len()));
        }
    } else {
        int_part.push('0');
        let zeros = (-exp - 1) as usize;
        frac_part.push_str(&"0".repeat(zeros));
        frac_part.push_str(&digits);
    }
    if frac_part.len() > frac_places {
        frac_part.truncate(frac_places);
    }
    let mut s = int_part.trim_start_matches('0').to_string();
    if s.is_empty() {
        s.push('0');
    }
    if frac_places > 0 {
        let f = frac_part.trim_end_matches('0');
        if !f.is_empty() {
            s.push('.');
            s.push_str(f);
        }
    }
    s
}

fn general_sci(mant: u64, exp: i32, budget: usize) -> String {
    let digits = format!("{mant:015}");
    let exp_s = if exp.abs() >= 100 {
        format!("E{exp:+}")
    } else {
        format!("E{exp:+03}")
    };
    let mant_budget = budget.saturating_sub(exp_s.len()).max(1);
    let mut m = String::new();
    m.push(digits.as_bytes()[0] as char);
    let rest = &digits[1..];
    if mant_budget > 1 && rest.chars().any(|c| c != '0') {
        m.push('.');
        let take = mant_budget.saturating_sub(2).min(rest.len());
        let mut frac = rest[..take].to_string();
        frac = frac.trim_end_matches('0').to_string();
        if !frac.is_empty() {
            m.push_str(&frac);
        }
    }
    format!("{m}{exp_s}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_basics() {
        assert_eq!(general(0.0), "0");
        assert_eq!(general(-0.0), "0");
        assert_eq!(general(1.0), "1");
        assert_eq!(general(0.0001), "0.0001");
        assert_eq!(general(0.00001), "1E-05");
        assert_eq!(general(1e11), "1E+11");
    }
}
