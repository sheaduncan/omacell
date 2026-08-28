//! Nightly: stream a synthetic 1 GB CSV without holding a second full copy.

use std::io::{self, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use omacell_io::csv::{ImportPlan, LoadOptions, LoadProgress, load_into};

/// Repeating `x` * 1023 + newline, `MAX_ROWS` times (~1.07 GB).
struct OneGbCsv {
    rows_left: u32,
    in_row: usize,
}

impl OneGbCsv {
    fn new() -> Self {
        Self {
            rows_left: omacell_core::limits::MAX_ROWS,
            in_row: 0,
        }
    }
}

impl Read for OneGbCsv {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.rows_left == 0 || buf.is_empty() {
            return Ok(0);
        }
        let mut wrote = 0;
        while wrote < buf.len() && self.rows_left > 0 {
            if self.in_row < 1023 {
                let n = (1023 - self.in_row).min(buf.len() - wrote);
                buf[wrote..wrote + n].fill(b'x');
                wrote += n;
                self.in_row += n;
            } else {
                buf[wrote] = b'\n';
                wrote += 1;
                self.in_row = 0;
                self.rows_left -= 1;
            }
        }
        Ok(wrote)
    }
}

#[test]
#[ignore = "nightly: streams ~1 GB into the grid"]
fn load_one_gb_progressively() {
    let mut plan = ImportPlan::default();
    plan.columns.push(omacell_io::csv::ColumnPlan {
        name: None,
        ty: omacell_io::csv::ColumnType::Text,
    });
    let rows = Arc::new(AtomicU64::new(0));
    let rows2 = Arc::clone(&rows);
    let opts = LoadOptions {
        progress_every: 100_000,
        on_progress: Some(Arc::new(move |p: LoadProgress| {
            rows2.store(p.rows_loaded, Ordering::Relaxed);
        })),
        ..Default::default()
    };
    let mut wb = omacell_core::workbook::Workbook::new();
    let rss_before = rss_bytes();
    let started = Instant::now();
    let result = load_into(&mut wb, OneGbCsv::new(), &plan, opts).unwrap();
    let elapsed = started.elapsed();
    let rss_delta = rss_before
        .zip(rss_bytes())
        .map(|(before, after)| after.saturating_sub(before));
    assert!(!result.cancelled);
    assert_eq!(
        result.rows_written,
        u64::from(omacell_core::limits::MAX_ROWS)
    );
    assert_eq!(result.bytes_read, 1_073_741_824);
    assert!(rows.load(Ordering::Relaxed) > 0);
    if let Some(delta) = rss_delta {
        assert!(
            delta < 512 * 1024 * 1024,
            "streaming load retained {delta} bytes of RSS"
        );
    }
    let sheet = wb.active_sheet();
    let slot = wb.get(sheet, 0, 0).unwrap().unwrap();
    let omacell_core::value::Value::Text(id) = slot.value else {
        panic!("expected text");
    };
    assert_eq!(wb.intern().strings.get(id).unwrap().len(), 1023);
    eprintln!("1 GiB streamed in {elapsed:?}; RSS delta {rss_delta:?}");
}

fn rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(kib * 1024)
}
