//! Fuzz bounded embedded Lua parsing and execution under the production sandbox.
#![no_main]

use libfuzzer_sys::fuzz_target;
use omacell_bus::Bus;
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use omacell_lua::{BusHost, Profile, Runtime};

fuzz_target!(|data: &[u8]| {
    if data.len() > 4 * 1024 {
        return;
    }
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(bus) = Bus::new(Workbook::new(), RecalcEngine::new(FnRegistry::new())) else {
        return;
    };
    let Ok(runtime) = Runtime::new(Profile::Embedded, Box::new(BusHost::new(bus))) else {
        return;
    };
    let _ = runtime.exec(source, "fuzz.lua");
});
