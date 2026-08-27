#![no_main]

use libfuzzer_sys::fuzz_target;
use omacell_core::locale::LocaleId;
use omacell_core::numfmt::{format, parse, FormatValue};

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    let _ = parse(s);
    let loc = LocaleId::EN_US;
    let _ = format(FormatValue::Number(1234.5), s, loc);
    let _ = format(FormatValue::Number(-1234.5), s, loc);
    let _ = format(FormatValue::Number(0.0), s, loc);
    let _ = format(FormatValue::Number(-0.0), s, loc);
    let _ = format(FormatValue::Text("hello"), s, loc);
    let _ = format(FormatValue::Bool(true), s, loc);
});
