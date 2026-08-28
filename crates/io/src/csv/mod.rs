//! CSV/TSV import with preview, progressive load, and export (spec F-9.4).
//!
//! The AI import assistant (A-4.4) is a hook only: see [`import_assist_request`].
//!
//! ```
//! use omacell_io::csv::{load, sniff};
//! let bytes = b"1,2\n3,4\n";
//! let sniffed = sniff(bytes).unwrap();
//! let (wb, result) = load(bytes, &sniffed.plan, Default::default()).unwrap();
//! assert_eq!(result.rows_written, 2);
//! assert_eq!(
//!     wb.get(wb.active_sheet(), 0, 0).unwrap().unwrap().value,
//!     omacell_core::value::Value::Number(1.0)
//! );
//! ```

mod assist;
mod clipboard;
mod encode;
mod infer;
mod plan;
mod preview;
mod reader;
mod records;
mod sniff;
mod writer;

pub use assist::{ImportAssistRequest, import_assist_request};
pub use clipboard::{ClipboardFormat, ClipboardTable, parse_clipboard};
pub use encode::{bom_len, decode_all, detect_bom, encode_all, sniff_encoding};
pub use infer::{Converted, ConvertedKind, convert_cell};
pub use plan::{
    ColumnPlan, ColumnType, DEFAULT_PREVIEW_ROWS, ExportPlan, FormulaTextPolicy, ImportPlan,
    LineEnding, MAX_BUFFERED_EXPORT_BYTES, MAX_CLIPBOARD_BYTES, MAX_CLIPBOARD_CELLS,
    MAX_CLIPBOARD_ROWS, MAX_EXPORT_RECORD_BYTES, MAX_FIELD_BYTES, MAX_PREVIEW_ROWS,
    MAX_SNIFF_BYTES, Quoting, TextEncoding, ValueMode,
};
pub use preview::{PreviewCell, PreviewRows, preview, preview_path, preview_row};
pub use reader::{LoadOptions, LoadProgress, LoadResult, load, load_into, load_path};
pub use sniff::{Sniff, sniff, sniff_path};
pub use writer::{export, export_write};
