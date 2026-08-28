//! Fuzz smoke: CSV sniff / preview / load / clipboard over bounded payloads.
#![no_main]

use libfuzzer_sys::fuzz_target;
use omacell_io::csv::{
    ClipboardFormat, ImportPlan, MAX_SNIFF_BYTES, load, parse_clipboard, preview, sniff,
};

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_SNIFF_BYTES {
        return;
    }
    let plan = sniff(data)
        .map(|s| s.plan)
        .unwrap_or_else(|_| ImportPlan::default());
    let _ = preview(data, &plan, 8);
    let _ = load(data, &plan, Default::default());
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = parse_clipboard(text, ClipboardFormat::Auto);
    }
});
