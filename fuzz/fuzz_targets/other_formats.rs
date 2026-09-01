//! Fuzz smoke for the WP-27 materializing format parsers.
#![no_main]

use libfuzzer_sys::fuzz_target;
use omacell_io::csv::ClipboardFormat;

fuzz_target!(|data: &[u8]| {
    if data.len() > 64 * 1024 {
        return;
    }
    let _ = omacell_io::ods::open_bytes(data);
    let _ = omacell_io::json::open_bytes(data);
    let _ = omacell_io::html::open_bytes(data, ClipboardFormat::Html);
    let _ = omacell_io::html::open_bytes(data, ClipboardFormat::Markdown);
    let _ = omacell_io::parquet::open_bytes(data);
    let _ = omacell_io::bridge::open_xls_bytes(data);
});
