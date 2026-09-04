//! Progressive CSV load into a [`Workbook`].

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use omacell_core::addr::SheetId;
use omacell_core::error::CoreError;
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use omacell_core::storage::CellSlot;
use omacell_core::style::{NumFmtId, Style};
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
use serde::{Deserialize, Serialize};

use super::encode::{CountingReader, DecodingReader, bom_len};
use super::infer::{Converted, convert_cell};
use super::plan::ImportPlan;
use super::records::{RecordReader, check_record};
use crate::error;

/// Progress callback payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadProgress {
    /// Data rows written (excluding header).
    pub rows_loaded: u64,
    /// Encoded bytes consumed from the source.
    pub bytes_read: u64,
    /// True on the final callback.
    pub done: bool,
}

/// Options for [`load_into`].
#[derive(Clone)]
pub struct LoadOptions {
    /// 0-based origin row.
    pub origin_row: u32,
    /// 0-based origin column.
    pub origin_col: u16,
    /// Target sheet name; created if missing. Default: active sheet.
    pub sheet_name: Option<String>,
    /// Cooperative cancel flag.
    pub cancel: Option<Arc<AtomicBool>>,
    /// Invoked every [`Self::progress_every`] rows and at completion.
    pub on_progress: Option<Arc<dyn Fn(LoadProgress) + Send + Sync>>,
    /// Row interval for progress. `0` means only the final event.
    pub progress_every: u64,
}

impl std::fmt::Debug for LoadOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadOptions")
            .field("origin_row", &self.origin_row)
            .field("origin_col", &self.origin_col)
            .field("sheet_name", &self.sheet_name)
            .field("cancel", &self.cancel.is_some())
            .field("on_progress", &self.on_progress.is_some())
            .field("progress_every", &self.progress_every)
            .finish()
    }
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            origin_row: 0,
            origin_col: 0,
            sheet_name: None,
            cancel: None,
            on_progress: None,
            progress_every: 10_000,
        }
    }
}

/// Outcome of a load.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadResult {
    /// Sheet written.
    pub sheet: SheetId,
    /// Data rows written (header not counted).
    pub rows_written: u64,
    /// Widest row (column count).
    pub cols: u16,
    /// True when [`LoadOptions::cancel`] fired.
    pub cancelled: bool,
    /// Encoded bytes consumed.
    pub bytes_read: u64,
}

/// Load `bytes` into a new workbook.
pub fn load(
    bytes: &[u8],
    plan: &ImportPlan,
    opts: LoadOptions,
) -> Result<(Workbook, LoadResult), CoreError> {
    let mut wb = Workbook::new();
    let result = load_into(&mut wb, bytes, plan, opts)?;
    Ok((wb, result))
}

/// Load a path into a new workbook.
pub fn load_path(
    path: &Path,
    plan: &ImportPlan,
    opts: LoadOptions,
) -> Result<(Workbook, LoadResult), CoreError> {
    let mut wb = Workbook::new();
    let file = std::fs::File::open(path).map_err(|e| error::parse(e.to_string()))?;
    let result = load_reader(&mut wb, file, plan, opts)?;
    Ok((wb, result))
}

/// Load decoded-or-raw `bytes` into `wb`.
pub fn load_into<R: Read>(
    wb: &mut Workbook,
    reader: R,
    plan: &ImportPlan,
    opts: LoadOptions,
) -> Result<LoadResult, CoreError> {
    load_reader(wb, reader, plan, opts)
}

fn load_reader<R: Read>(
    wb: &mut Workbook,
    reader: R,
    plan: &ImportPlan,
    opts: LoadOptions,
) -> Result<LoadResult, CoreError> {
    plan.validate()?;
    let sheet = resolve_sheet(wb, opts.sheet_name.as_deref())?;
    let counted = CountingReader::new(reader);
    // CountingReader is moved into DecodingReader; recover bytes via a cell.
    let bytes = std::rc::Rc::new(std::cell::Cell::new(0u64));
    let counted = ByteTap {
        inner: counted,
        tap: bytes.clone(),
    };
    let mut buffered = BufReader::new(counted);
    let skip = bom_len(
        plan.encoding,
        buffered
            .fill_buf()
            .map_err(|e| error::parse(e.to_string()))?,
    );
    let decoded = DecodingReader::new(buffered, plan.encoding, skip);
    let mut rdr = RecordReader::new(decoded, plan)?;

    let undo_was = wb.undo_log().is_enabled();
    wb.undo_log_mut().set_enabled(false);

    let result = load_records(wb, sheet, &mut rdr, plan, &opts, &bytes);
    wb.undo_log_mut().set_enabled(undo_was);
    result
}

struct ByteTap<R: Read> {
    inner: CountingReader<R>,
    tap: std::rc::Rc<std::cell::Cell<u64>>,
}

impl<R: Read> Read for ByteTap<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.tap.set(self.inner.bytes);
        Ok(n)
    }
}

fn resolve_sheet(wb: &mut Workbook, name: Option<&str>) -> Result<SheetId, CoreError> {
    match name {
        None => Ok(wb.active_sheet()),
        Some(n) => {
            if let Ok(id) = wb.resolve_sheet_name(n) {
                Ok(id)
            } else {
                wb.add_sheet(n)
            }
        }
    }
}

fn load_records<R: Read>(
    wb: &mut Workbook,
    sheet: SheetId,
    rdr: &mut RecordReader<R>,
    plan: &ImportPlan,
    opts: &LoadOptions,
    bytes: &std::rc::Rc<std::cell::Cell<u64>>,
) -> Result<LoadResult, CoreError> {
    let mut rec_index: u64 = 0;
    let mut data_row: u64 = 0;
    let mut cols: u16 = 0;
    let mut cancelled = false;
    let origin_row = opts.origin_row;
    let origin_col = opts.origin_col;

    let mut rec = ::csv::StringRecord::new();
    while rdr.read_record(&mut rec)? {
        if opts
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
        {
            cancelled = true;
            break;
        }
        check_record(&rec)?;
        if rec_index < u64::from(plan.skip_rows) {
            rec_index += 1;
            continue;
        }
        let is_header = plan.has_header && rec_index == u64::from(plan.skip_rows);
        rec_index += 1;

        if rec.len() > usize::from(MAX_COLS) {
            return Err(error::limit(format!(
                "row has {} fields; maximum is {MAX_COLS}",
                rec.len()
            )));
        }
        let width = u16::try_from(rec.len()).unwrap_or(MAX_COLS);
        cols = cols.max(width);

        let grid_row = if is_header {
            origin_row
        } else {
            let offset = data_row + u64::from(plan.has_header);
            let row = u64::from(origin_row) + offset;
            if row >= u64::from(MAX_ROWS) {
                return Err(error::limit(format!(
                    "row {} is outside the Excel grid",
                    row + 1
                )));
            }
            row as u32
        };

        for (i, field) in rec.iter().enumerate() {
            let col = u32::from(origin_col) + i as u32;
            if col >= u32::from(MAX_COLS) {
                return Err(error::limit("column is outside the Excel grid"));
            }
            let col = col as u16;
            if is_header {
                if !field.is_empty() {
                    wb.set_text(sheet, grid_row, col, field)?;
                }
                continue;
            }
            write_cell(wb, sheet, grid_row, col, field, i, plan)?;
        }

        if !is_header {
            data_row += 1;
            emit_progress(opts, data_row, bytes.get(), false);
        }
    }

    let result = LoadResult {
        sheet,
        rows_written: data_row,
        cols,
        cancelled,
        bytes_read: bytes.get(),
    };
    emit_progress(opts, result.rows_written, result.bytes_read, true);
    if cancelled {
        return Err(error::cancelled(format!(
            "cancelled after {} rows",
            result.rows_written
        )));
    }
    Ok(result)
}

fn emit_progress(opts: &LoadOptions, rows: u64, bytes: u64, done: bool) {
    let Some(cb) = opts.on_progress.as_ref() else {
        return;
    };
    if !done && (opts.progress_every == 0 || !rows.is_multiple_of(opts.progress_every)) {
        return;
    }
    cb(LoadProgress {
        rows_loaded: rows,
        bytes_read: bytes,
        done,
    });
}

fn write_cell(
    wb: &mut Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
    raw: &str,
    col_idx: usize,
    plan: &ImportPlan,
) -> Result<(), CoreError> {
    if raw.is_empty() {
        return Ok(());
    }
    // Never treat CSV as formula source (F-9.6).
    let converted = convert_cell(raw, plan.column_type(col_idx), plan);
    match converted {
        Converted::Empty => Ok(()),
        Converted::Number(n) => {
            wb.set_number(sheet, row, col, n)?;
            Ok(())
        }
        Converted::Bool(b) => {
            wb.set_slot(
                sheet,
                row,
                col,
                CellSlot {
                    value: Value::Bool(b),
                    formula: None,
                    style: omacell_core::style::StyleId::DEFAULT,
                    flags: omacell_core::storage::CellFlags::DEFAULT,
                },
            )?;
            Ok(())
        }
        Converted::Date { serial, num_fmt } => {
            wb.set_number(sheet, row, col, serial)?;
            let style = Style {
                num_fmt: NumFmtId::new(num_fmt),
                ..Style::default()
            };
            wb.set_cell_style(sheet, row, col, style)?;
            Ok(())
        }
        Converted::Text(s) => {
            wb.set_text(sheet, row, col, &s)?;
            Ok(())
        }
    }
}
