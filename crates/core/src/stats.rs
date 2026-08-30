//! Descriptive statistics for a selection (F-7.3).

use serde::{Deserialize, Serialize};

use crate::addr::{RangeRef, SheetId};
use crate::error::CoreError;
use crate::value::Value;
use crate::workbook::Workbook;

/// Summary of a range.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatsSummary {
    /// Count of numeric cells.
    pub count: u32,
    /// Count of non-empty cells.
    pub count_a: u32,
    /// Sum of numbers.
    pub sum: f64,
    /// Arithmetic mean, if `count > 0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean: Option<f64>,
    /// Sample standard deviation, if `count >= 2`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdev: Option<f64>,
    /// Sample variance, if `count >= 2`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub var: Option<f64>,
    /// Minimum number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Maximum number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Median.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub median: Option<f64>,
    /// Histogram bins `(start, end, count)` covering `[min, max]`.
    pub histogram: Vec<HistBin>,
}

/// One histogram bin.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistBin {
    /// Inclusive start.
    pub start: f64,
    /// Exclusive end (inclusive on the last bin).
    pub end: f64,
    /// Count.
    pub count: u32,
}

/// Describe numeric cells in `range`.
pub fn describe_range(
    wb: &Workbook,
    sheet: SheetId,
    range: RangeRef,
) -> Result<StatsSummary, CoreError> {
    if wb.sheet(sheet).is_none() {
        return Err(CoreError::sheet_id(format!(
            "unknown sheet {}",
            sheet.index()
        )));
    }
    let r0 = range.start.row.min(range.end.row);
    let r1 = range.start.row.max(range.end.row);
    let c0 = range.start.col.min(range.end.col);
    let c1 = range.start.col.max(range.end.col);
    let mut nums = Vec::new();
    let mut count_a = 0u32;
    for r in r0..=r1 {
        for c in c0..=c1 {
            match wb.get(sheet, r, c).ok().flatten().map(|s| s.value) {
                Some(Value::Number(n)) if n.is_finite() => {
                    count_a += 1;
                    nums.push(n);
                }
                Some(Value::Bool(_)) | Some(Value::Text(_)) | Some(Value::Error(_)) => {
                    count_a += 1;
                }
                _ => {}
            }
        }
    }
    nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let count = u32::try_from(nums.len()).unwrap_or(u32::MAX);
    let sum: f64 = nums.iter().sum();
    let mean = (count > 0).then_some(sum / f64::from(count));
    let (stdev, var) = if nums.len() >= 2 {
        let m = mean.unwrap_or(0.0);
        let m2: f64 = nums
            .iter()
            .map(|x| {
                let d = x - m;
                d * d
            })
            .sum();
        let v = m2 / (nums.len() - 1) as f64;
        (Some(v.sqrt()), Some(v))
    } else {
        (None, None)
    };
    let min = nums.first().copied();
    let max = nums.last().copied();
    let median = median(&nums);
    let histogram = histogram(&nums);
    Ok(StatsSummary {
        count,
        count_a,
        sum,
        mean,
        stdev,
        var,
        min,
        max,
        median,
        histogram,
    })
}

fn median(nums: &[f64]) -> Option<f64> {
    if nums.is_empty() {
        return None;
    }
    let n = nums.len();
    if n % 2 == 1 {
        Some(nums[n / 2])
    } else {
        Some((nums[n / 2 - 1] + nums[n / 2]) / 2.0)
    }
}

fn histogram(nums: &[f64]) -> Vec<HistBin> {
    if nums.is_empty() {
        return Vec::new();
    }
    let min = nums[0];
    let max = nums[nums.len() - 1];
    if (max - min).abs() < 1e-15 {
        return vec![HistBin {
            start: min,
            end: min,
            count: u32::try_from(nums.len()).unwrap_or(u32::MAX),
        }];
    }
    let k = ((nums.len() as f64).log2().ceil() as usize + 1).clamp(1, 20);
    let width = (max - min) / k as f64;
    let mut bins = vec![0u32; k];
    for n in nums {
        let mut i = ((*n - min) / width).floor() as usize;
        if i >= k {
            i = k - 1;
        }
        bins[i] += 1;
    }
    bins.into_iter()
        .enumerate()
        .map(|(i, count)| HistBin {
            start: min + i as f64 * width,
            end: if i + 1 == k {
                max
            } else {
                min + (i + 1) as f64 * width
            },
            count,
        })
        .collect()
}
