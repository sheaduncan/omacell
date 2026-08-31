#![no_main]

use libfuzzer_sys::fuzz_target;
use omacell_conf::{ColorsToml, Config};
use omacell_lua::TrustStore;
use omacell_ui::Keymap;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = toml::from_str::<Config>(text);
        let _ = ColorsToml::parse(text);
        let _ = Keymap::parse(text);
        let _ = TrustStore::parse(text);
    }
});
