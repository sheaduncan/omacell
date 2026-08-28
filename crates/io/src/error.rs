//! Stable `{code, message, hint}` codes for workbook I/O.

use omacell_core::error::CoreError;

/// Machine codes for I/O errors. CLI, IPC, and MCP mirror these strings.
pub mod codes {
    /// Bytes are not valid in the selected encoding, or the encoding is unknown.
    pub const CSV_ENCODING: &str = "csv.encoding";
    /// Delimited text could not be parsed (quoting, records, or I/O).
    pub const CSV_PARSE: &str = "csv.parse";
    /// [`crate::csv::ImportPlan`] / [`crate::csv::ExportPlan`] is inconsistent.
    pub const CSV_PLAN: &str = "csv.plan";
    /// A size, row, column, or field limit was exceeded.
    pub const CSV_LIMIT: &str = "csv.limit";
    /// Progressive load was cancelled.
    pub const CSV_CANCELLED: &str = "csv.cancelled";
    /// Export failed (range, encoding, or quoting policy).
    pub const CSV_EXPORT: &str = "csv.export";
}

/// Encoding error.
#[must_use]
pub fn encoding(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::CSV_ENCODING, message)
        .with_hint("use UTF-8, UTF-16, or Latin-1; check the BOM and ImportPlan.encoding")
}

/// Parse error.
#[must_use]
pub fn parse(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::CSV_PARSE, message).with_hint("check delimiter, quoting, and encoding")
}

/// Plan error.
#[must_use]
pub fn plan(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::CSV_PLAN, message)
        .with_hint("delimiter and quote must be ASCII; decimal and thousands must differ")
}

/// Limit error.
#[must_use]
pub fn limit(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::CSV_LIMIT, message)
        .with_hint("narrow the grid request or use a streaming API for large files")
}

/// Cancelled load.
#[must_use]
pub fn cancelled(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::CSV_CANCELLED, message)
        .with_hint("partial rows remain in the workbook; retry or discard")
}

/// Export error.
#[must_use]
pub fn export(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::CSV_EXPORT, message)
        .with_hint("check the range, quoting, formula-text policy, encoding, and destination")
}
