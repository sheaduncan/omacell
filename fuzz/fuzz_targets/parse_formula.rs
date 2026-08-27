#![no_main]

use libfuzzer_sys::fuzz_target;
use omacell_core::formula::{parse, parse_editor};

fuzz_target!(|data: &[u8]| {
    if data.len() > 16_384 {
        return;
    }
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = parse(s);
        let _ = parse_editor(s);
    }
});
