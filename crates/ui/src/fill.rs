//! Fill-handle series detection (F-5.5).

/// How a fill should be interpreted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillKind {
    /// Copy the source.
    Copy,
    /// Arithmetic series.
    Linear,
    /// Date serial +1.
    Date,
}

/// Detect a series from the source column/row of numbers (as f64).
#[must_use]
pub fn detect_series(values: &[f64]) -> FillKind {
    if values.len() < 2 {
        return FillKind::Copy;
    }
    let step = values[1] - values[0];
    if step.abs() < 1e-9 {
        return FillKind::Copy;
    }
    if values.windows(2).all(|w| (w[1] - w[0] - step).abs() < 1e-9) {
        if (step.abs() - 1.0).abs() < 1e-9
            && values
                .iter()
                .all(|v| v.is_finite() && v.fract().abs() < 1e-9)
        {
            FillKind::Date
        } else {
            FillKind::Linear
        }
    } else {
        FillKind::Copy
    }
}

/// Next `n` values of a detected series.
#[must_use]
pub fn extend_series(values: &[f64], kind: FillKind, n: usize) -> Vec<f64> {
    let Some(&last) = values.last() else {
        return vec![0.0; n];
    };
    match kind {
        FillKind::Copy => vec![last; n],
        FillKind::Linear | FillKind::Date => {
            let step = if values.len() >= 2 {
                values[1] - values[0]
            } else {
                1.0
            };
            (1..=n).map(|i| last + step * i as f64).collect()
        }
    }
}
