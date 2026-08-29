//! Fuzz smoke: clap parse of argv never panics.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 4096 {
        return;
    }
    let mut args = vec!["omacell".to_string()];
    for chunk in data.split(|b| *b == 0) {
        if chunk.is_empty() {
            continue;
        }
        if let Ok(s) = std::str::from_utf8(chunk) {
            if s.chars().all(|c| !c.is_control()) {
                args.push(s.to_string());
            }
        }
        if args.len() > 16 {
            break;
        }
    }
    let _ = omacell_cli::try_parse(args);
});
