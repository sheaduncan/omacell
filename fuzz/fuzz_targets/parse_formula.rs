#![no_main]

use libfuzzer_sys::fuzz_target;
use omacell_core::formula::{parse, parse_editor};
use omacell_core::limits::MAX_FORMULA_LEN;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_FORMULA_LEN {
        return;
    }
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = parse(s);
        let _ = parse_editor(s);
    }
});
