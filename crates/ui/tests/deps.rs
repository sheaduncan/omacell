//! Toolkit types must not appear in this crate.

#[test]
fn cargo_toml_has_no_toolkit_deps() {
    let toml = include_str!("../Cargo.toml");
    for needle in ["egui", "ratatui", "winit", "eframe", "crossterm"] {
        assert!(
            !toml.contains(needle),
            "omacell-ui Cargo.toml must not mention {needle}"
        );
    }
}
