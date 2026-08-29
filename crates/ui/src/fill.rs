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
    if values.windows(2).all(|w| (w[1] - w[0] - step).abs() < 1e-9) {
        if step.abs() - 1.0 < 1e-9 && values.iter().all(|v| v.fract().abs() < 1e-9) {
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
    if values.is_empty() {
        return vec![0.0; n];
    }
    match kind {
        FillKind::Copy => vec![*values.last().unwrap(); n],
        FillKind::Linear | FillKind::Date => {
            let step = if values.len() >= 2 {
                values[1] - values[0]
            } else {
                1.0
            };
            let last = *values.last().unwrap();
            (1..=n).map(|i| last + step * i as f64).collect()
        }
    }
}
