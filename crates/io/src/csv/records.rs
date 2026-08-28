//! Shared csv-crate reader/writer configuration.

use std::fmt;
use std::io::{self, Read};

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
}

impl<R: Read> FieldLimitReader<R> {
    pub(crate) fn new(inner: R, plan: &ImportPlan) -> Result<Self, CoreError> {
        Ok(Self {
            inner,
            delimiter: plan.delimiter_byte()?,
            quote: plan.quote_byte()?,
            field_bytes: 0,
            fields_in_record: 1,
            at_field_start: true,
            in_quotes: false,
            quote_pending: false,
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

    fn observe_outside(&mut self, byte: u8) -> io::Result<()> {
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
        } else if matches!(byte, b'\r' | b'\n') {
            self.field_bytes = 0;
            self.fields_in_record = 1;
            self.at_field_start = true;
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
    let limited = FieldLimitReader::new(utf8, plan)?;
    let mut rdr = reader_builder(plan)?.from_reader(limited);
    collect_records(&mut rdr)
}

pub(crate) fn collect_records<R: Read>(
    rdr: &mut ::csv::Reader<R>,
) -> Result<Vec<Vec<String>>, CoreError> {
    let mut rows = Vec::new();
    let mut cells = 0usize;
    for rec in rdr.records() {
        let rec = rec.map_err(map_csv)?;
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
