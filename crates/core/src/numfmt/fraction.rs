//! Fraction formats (`# ?/?`, `# ??/??`, fixed denominators).

use crate::numfmt::number::round_half_away;

/// Best rational approximation of `x` in `[0, 1]` with denominator `<= max_den`.
#[must_use]
pub fn best_rational(x: f64, max_den: u32) -> (u32, u32) {
    if !x.is_finite() || x <= 0.0 {
        return (0, 1);
    }
    if x >= 1.0 {
        return (1, 1);
    }
    let max_den = max_den.max(1);
    let mut a = 0u32;
    let mut b = 1u32;
    let mut c = 1u32;
    let mut d = 1u32;
    while b + d <= max_den {
        let med_n = a + c;
        let med_d = b + d;
        let med = f64::from(med_n) / f64::from(med_d);
        if (med - x).abs() < 1e-15 {
            return (med_n, med_d);
        }
        if x > med {
            a = med_n;
            b = med_d;
        } else {
            c = med_n;
            d = med_d;
        }
    }
    let err_b = (f64::from(a) / f64::from(b) - x).abs();
    let err_d = (f64::from(c) / f64::from(d) - x).abs();
    if err_b <= err_d {
        (a, b)
    } else {
        (c, d)
    }
}

/// Format a mixed or improper fraction.
pub fn render_fraction(
    n: f64,
    _int_placeholders: usize,
    num_placeholders: usize,
    den_placeholders: usize,
    fixed_den: Option<u32>,
    mixed: bool,
) -> String {
    let neg = n < 0.0;
    let a = n.abs();
    if !a.is_finite() {
        return "#NUM!".to_string();
    }
    let mut whole = a.floor() as u64;
    let frac = a - a.floor();
    let max_den = fixed_den.unwrap_or(match den_placeholders {
        0 | 1 => 9,
        2 => 99,
        _ => 999,
    });
    let (mut num, den) = if let Some(d) = fixed_den {
        let d = d.max(1);
        let num = round_half_away(frac * f64::from(d), 0).max(0.0) as u32;
        (num, d)
    } else {
        best_rational(frac, max_den)
    };
    if den > 0 && num >= den {
        whole += u64::from(num / den);
        num %= den;
    }
    let mut body = String::new();
    if mixed {
        if whole > 0 {
            body.push_str(&whole.to_string());
            if num > 0 {
                body.push(' ');
            }
        } else if num == 0 {
            body.push('0');
        }
        if num > 0 {
            body.push_str(&pad_num(num, num_placeholders));
            body.push('/');
            body.push_str(&pad_num(den, den_placeholders));
        }
    } else {
        let improper = whole * u64::from(den) + u64::from(num);
        if improper == 0 {
            body.push('0');
        } else {
            body.push_str(&pad_num(improper as u32, num_placeholders.max(1)));
            body.push('/');
            body.push_str(&pad_num(den, den_placeholders.max(1)));
        }
    }
    if neg && body != "0" {
        format!("-{body}")
    } else {
        body
    }
}

fn pad_num(n: u32, width: usize) -> String {
    let s = n.to_string();
    if width <= 1 || s.len() >= width {
        s
    } else {
        format!("{n:width$}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves() {
        assert_eq!(best_rational(0.5, 9), (1, 2));
        assert_eq!(render_fraction(1.5, 1, 1, 1, None, true), "1 1/2");
        assert_eq!(render_fraction(0.5, 0, 1, 1, Some(8), true), "4/8");
    }
}
