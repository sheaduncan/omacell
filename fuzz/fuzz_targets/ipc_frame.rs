//! Fuzz smoke: IPC v1 frame decoder over bounded byte payloads.
#![no_main]

use libfuzzer_sys::fuzz_target;
use omacell_bus::ipc::{MAX_FRAME_BYTES, decode_request_bytes};

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_FRAME_BYTES + 32 {
        return;
    }
    let _ = decode_request_bytes(data);
});
