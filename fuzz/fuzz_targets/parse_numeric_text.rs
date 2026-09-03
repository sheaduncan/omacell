#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &str| {
    let _ = omacell_core::coerce::parse_numeric_text(input);
});
