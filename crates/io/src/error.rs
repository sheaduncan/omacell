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
    /// Zip container is malformed or not an OPC package.
    pub const XLSX_ZIP: &str = "xlsx.zip";
    /// A zip/XML size, depth, count, or compression-ratio limit was exceeded.
    pub const XLSX_LIMIT: &str = "xlsx.limit";
    /// XML is malformed, too deep, or contains a DTD/entity.
    pub const XLSX_XML: &str = "xlsx.xml";
    /// The package is not a readable workbook (missing parts or broken rels).
    pub const XLSX_FORMAT: &str = "xlsx.format";
    /// A zip entry path is absolute, has `..`, or is otherwise illegal.
    pub const XLSX_PATH: &str = "xlsx.path";
    /// Writing or encoding an `.xlsx` package failed.
    pub const XLSX_WRITE: &str = "xlsx.write";
    /// A LibreOffice-compatible lock file blocked the save.
    pub const XLSX_LOCK: &str = "xlsx.lock";
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

/// Zip container error.
#[must_use]
pub fn xlsx_zip(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::XLSX_ZIP, message).with_hint("the file is not a readable OPC zip package")
}

/// Zip/XML limit error.
#[must_use]
pub fn xlsx_limit(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::XLSX_LIMIT, message)
        .with_hint("parsers reject zip bombs, deep XML, and oversized parts (spec F-9.6)")
}

/// XML error.
#[must_use]
pub fn xlsx_xml(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::XLSX_XML, message)
        .with_hint("external entities are disabled; XML depth is capped")
}

/// Workbook structure error.
#[must_use]
pub fn xlsx_format(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::XLSX_FORMAT, message)
        .with_hint("the package must contain a workbook part and worksheet relationships")
}

/// Illegal zip path.
#[must_use]
pub fn xlsx_path(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::XLSX_PATH, message)
        .with_hint("zip entries cannot be absolute or contain '..'")
}

/// Write error.
#[must_use]
pub fn xlsx_write(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::XLSX_WRITE, message)
        .with_hint("check free space and that the destination directory is writable")
}

/// Lock error.
#[must_use]
pub fn xlsx_lock(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::XLSX_LOCK, message)
        .with_hint("close the other editor or remove a stale .~lock.*# file")
}
