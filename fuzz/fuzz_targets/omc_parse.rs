//! Fuzz smoke: `.omc` reader over bounded payloads.
#![no_main]

use libfuzzer_sys::fuzz_target;
use omacell_io::omc::{MAX_OMC_BYTES, open_bytes};

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_OMC_BYTES.min(64 * 1024) {
        return;
    }
    let _ = open_bytes(data);
});
