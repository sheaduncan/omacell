//! Fuzz smoke: OPC zip + xlsx open over bounded payloads.
#![no_main]

use libfuzzer_sys::fuzz_target;
use omacell_io::xlsx::open_bytes;

fuzz_target!(|data: &[u8]| {
    if data.len() > 64 * 1024 {
        return;
    }
    let _ = open_bytes(data);
});
