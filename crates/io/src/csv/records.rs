//! Shared csv-crate reader/writer configuration.

use std::fmt;
use std::io::{self, Read};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
};

use omacell_core::error::CoreError;
use omacell_core::limits::MAX_COLS;

use super::plan::{ImportPlan, MAX_CLIPBOARD_CELLS, MAX_CLIPBOARD_ROWS, MAX_FIELD_BYTES};
use crate::error;

const READER_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug)]
struct CsvLimitExceeded {
    message: String,
}

impl fmt::Display for CsvLimitExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CsvLimitExceeded {}

/// Streaming guard that stops the CSV parser before a field can grow without
/// bound. It counts decoded UTF-8 bytes and understands quoted delimiters,
/// quoted newlines, and doubled quote escapes.
pub(crate) struct FieldLimitReader<R: Read> {
    inner: R,
    delimiter: u8,
    quote: u8,
    field_bytes: usize,
    fields_in_record: usize,
    at_field_start: bool,
    in_quotes: bool,
    quote_pending: bool,
    last_terminator_was_cr: bool,
    real_records_seen: u64,
    blank_runs: Rc<BlankRuns>,
}

#[derive(Clone, Copy)]
struct BlankRun {
    before_record: u64,
    count: usize,
}

#[derive(Default)]
struct BlankRuns {
    queue: RefCell<VecDeque<BlankRun>>,
    available: Cell<bool>,
}

impl<R: Read> FieldLimitReader<R> {
    fn new(inner: R, plan: &ImportPlan, blank_runs: Rc<BlankRuns>) -> Result<Self, CoreError> {
        Ok(Self {
            inner,
            delimiter: plan.delimiter_byte()?,
            quote: plan.quote_byte()?,
            field_bytes: 0,
            fields_in_record: 1,
            at_field_start: true,
            in_quotes: false,
            quote_pending: false,
            last_terminator_was_cr: false,
            real_records_seen: 0,
            blank_runs,
        })
    }

    fn add_field_byte(&mut self) -> io::Result<()> {
        self.field_bytes += 1;
        if self.field_bytes > MAX_FIELD_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                CsvLimitExceeded {
                    message: format!(
                        "field is more than {MAX_FIELD_BYTES} bytes (observed at least {})",
                        self.field_bytes
                    ),
                },
            ));
        }
        Ok(())
    }

    fn add_blank_record(&mut self) -> io::Result<()> {
        let mut runs = self.blank_runs.queue.borrow_mut();
        if let Some(run) = runs
            .back_mut()
            .filter(|run| run.before_record == self.real_records_seen)
        {
            run.count = run.count.checked_add(1).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "blank record overflow")
            })?;
            return Ok(());
        }
        runs.push_back(BlankRun {
            before_record: self.real_records_seen,
            count: 1,
        });
        self.blank_runs.available.set(true);
        Ok(())
    }

    fn observe_outside(&mut self, byte: u8) -> io::Result<()> {
        if matches!(byte, b'\r' | b'\n') {
            let physical_blank =
                self.fields_in_record == 1 && self.field_bytes == 0 && self.at_field_start;
            if byte == b'\n' && self.last_terminator_was_cr && physical_blank {
                self.last_terminator_was_cr = false;
                return Ok(());
            }
            if physical_blank {
                self.add_blank_record()?;
            } else {
                self.real_records_seen =
                    self.real_records_seen.checked_add(1).ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "CSV record count overflow")
                    })?;
            }
            self.field_bytes = 0;
            self.fields_in_record = 1;
            self.at_field_start = true;
            self.last_terminator_was_cr = byte == b'\r';
            return Ok(());
        }

        if byte == self.delimiter {
            self.field_bytes = 0;
            self.at_field_start = true;
            self.fields_in_record += 1;
            if self.fields_in_record > usize::from(MAX_COLS) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    CsvLimitExceeded {
                        message: format!(
                            "record has more than {MAX_COLS} fields (observed at least {})",
                            self.fields_in_record
                        ),
                    },
                ));
            }
        } else if self.at_field_start && byte == self.quote {
            self.at_field_start = false;
            self.in_quotes = true;
        } else {
            self.at_field_start = false;
            self.add_field_byte()?;
        }
        Ok(())
    }

    fn observe(&mut self, byte: u8) -> io::Result<()> {
        if !self.in_quotes {
            return self.observe_outside(byte);
        }
        if self.quote_pending {
            self.quote_pending = false;
            if byte == self.quote {
                return self.add_field_byte();
            }
            self.in_quotes = false;
            return self.observe_outside(byte);
        }
        if byte == self.quote {
            self.quote_pending = true;
            Ok(())
        } else {
            self.add_field_byte()
        }
    }
}

impl<R: Read> Read for FieldLimitReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        for byte in &buf[..read] {
            self.observe(*byte)?;
        }
        Ok(read)
    }
}

/// CSV record reader that restores non-trailing physical blank records, which
/// the underlying parser intentionally skips.
pub(crate) struct RecordReader<R: Read> {
    reader: ::csv::Reader<FieldLimitReader<R>>,
    blank_runs: Rc<BlankRuns>,
    held_record: Option<::csv::StringRecord>,
    blanks_before_held: usize,
}

impl<R: Read> RecordReader<R> {
    pub(crate) fn new(inner: R, plan: &ImportPlan) -> Result<Self, CoreError> {
        let blank_runs = Rc::new(BlankRuns::default());
        let limited = FieldLimitReader::new(inner, plan, Rc::clone(&blank_runs))?;
        let reader = reader_builder(plan)?.from_reader(limited);
        Ok(Self {
            reader,
            blank_runs,
            held_record: None,
            blanks_before_held: 0,
        })
    }

    pub(crate) fn read_record(
        &mut self,
        record: &mut ::csv::StringRecord,
    ) -> Result<bool, CoreError> {
        if self.blanks_before_held > 0 {
            record.clear();
            record.push_field("");
            self.blanks_before_held -= 1;
            return Ok(true);
        }
        if let Some(held) = self.held_record.take() {
            *record = held;
            return Ok(true);
        }

        if !self.reader.read_record(record).map_err(map_csv)? {
            return Ok(false);
        }
        if !self.blank_runs.available.get() {
            return Ok(true);
        }
        let records_read = self.reader.position().record();
        let mut runs = self.blank_runs.queue.borrow_mut();
        while runs
            .front()
            .is_some_and(|run| run.before_record < records_read)
        {
            let Some(run) = runs.pop_front() else {
                break;
            };
            self.blanks_before_held = self
                .blanks_before_held
                .checked_add(run.count)
                .ok_or_else(|| error::limit("blank record count overflow"))?;
        }
        self.blank_runs.available.set(!runs.is_empty());
        drop(runs);

        if self.blanks_before_held > 0 {
            self.held_record = Some(std::mem::take(record));
            record.push_field("");
            self.blanks_before_held -= 1;
        }
        Ok(true)
    }
}

pub(crate) fn reader_builder(plan: &ImportPlan) -> Result<::csv::ReaderBuilder, CoreError> {
    plan.validate()?;
    let mut b = ::csv::ReaderBuilder::new();
    b.delimiter(plan.delimiter_byte()?)
        .quote(plan.quote_byte()?)
        .has_headers(false)
        .flexible(true)
        .terminator(::csv::Terminator::CRLF)
        .buffer_capacity(READER_BUFFER_BYTES);
    Ok(b)
}

/// Parse every record from already-decoded UTF-8 bytes.
pub(crate) fn parse_records(utf8: &[u8], plan: &ImportPlan) -> Result<Vec<Vec<String>>, CoreError> {
    let mut rdr = RecordReader::new(utf8, plan)?;
    collect_records(&mut rdr)
}

pub(crate) fn collect_records<R: Read>(
    rdr: &mut RecordReader<R>,
) -> Result<Vec<Vec<String>>, CoreError> {
    let mut rows = Vec::new();
    let mut cells = 0usize;
    let mut rec = ::csv::StringRecord::new();
    while rdr.read_record(&mut rec)? {
        if rows.len() >= MAX_CLIPBOARD_ROWS {
            return Err(error::limit(format!(
                "materialized table has more than {MAX_CLIPBOARD_ROWS} rows"
            )));
        }
        cells = cells
            .checked_add(rec.len())
            .ok_or_else(|| error::limit("materialized table cell count overflow"))?;
        if cells > MAX_CLIPBOARD_CELLS {
            return Err(error::limit(format!(
                "materialized table has more than {MAX_CLIPBOARD_CELLS} cells"
            )));
        }
        if rec.len() == 1 && rec.get(0).is_some_and(|s| s.is_empty()) && rows.is_empty() {
            // keep empty records; a trailing newline after the last row is
            // an empty record we drop at the end instead.
        }
        rows.push(record_to_row(&rec)?);
    }
    while rows.last().is_some_and(|r| r.len() == 1 && r[0].is_empty()) {
        rows.pop();
    }
    Ok(rows)
}

pub(crate) fn check_record(rec: &::csv::StringRecord) -> Result<(), CoreError> {
    if rec.len() > usize::from(MAX_COLS) {
        return Err(error::limit(format!(
            "record has {} fields; maximum is {MAX_COLS}",
            rec.len()
        )));
    }
    if let Some(field) = rec.iter().find(|field| field.len() > MAX_FIELD_BYTES) {
        return Err(error::limit(format!(
            "field is {} bytes; maximum is {MAX_FIELD_BYTES}",
            field.len()
        )));
    }
    Ok(())
}

pub(crate) fn record_to_row(rec: &::csv::StringRecord) -> Result<Vec<String>, CoreError> {
    check_record(rec)?;
    Ok(rec.iter().map(ToOwned::to_owned).collect())
}

pub(crate) fn map_csv(err: ::csv::Error) -> CoreError {
    if let ::csv::ErrorKind::Io(io_error) = err.kind()
        && io_error
            .get_ref()
            .is_some_and(|inner| inner.is::<CsvLimitExceeded>())
    {
        return error::limit(io_error.to_string());
    }
    error::parse(err.to_string())
}
