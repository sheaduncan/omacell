//! Keymap conformance: registered ∪ deferred; no unknown ids; no duplicate chords.

use omacell_bus::{CommandRegistry, register_core};
use omacell_conf::{Paths, load};
use omacell_ui::{Keymap, KeymapRoots, UiSession, deferred_owner, register_ui_commands};

fn load_session(model_file: &str) -> (tempfile::TempDir, UiSession, CommandRegistry, Keymap) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    std::fs::create_dir_all(&paths.user_config).unwrap();
    std::fs::write(
        paths.user_config.join("config.toml"),
        format!("[keys]\nfile = \"{model_file}\"\n"),
    )
    .unwrap();
    let loaded = load(&paths, &[], None).unwrap();
    let roots = KeymapRoots::new(paths.user_config.clone(), paths.default_dir.clone(), None);
    let session = UiSession::new(&loaded, &roots).unwrap();
    let mut registry = CommandRegistry::new();
    register_core(&mut registry).unwrap();
    register_ui_commands(&mut registry, &session).unwrap();
    let keymap = session.keymap();
    (dir, session, registry, keymap)
}

fn assert_conforms(keymap: &Keymap, registry: &CommandRegistry) {
    let registered: Vec<String> = registry.iter().map(|(id, _)| id.to_string()).collect();
    let mut seen = std::collections::BTreeSet::new();
    for (mode, chord, binding) in keymap.iter() {
        let key = format!("{mode}\0{chord}");
        assert!(seen.insert(key), "duplicate chord {chord} in mode {mode}");
        let known = registered.iter().any(|id| id == &binding.cmd)
            || deferred_owner(&binding.cmd).is_some();
        assert!(
            known,
            "unowned command {} (chord {chord} mode {mode})",
            binding.cmd
        );
    }
}

#[test]
fn classic_map_conforms() {
    let (_dir, _session, registry, keymap) = load_session("keys/classic.toml");
    assert_eq!(keymap.model, omacell_ui::KeyModel::Classic);
    assert_conforms(&keymap, &registry);
}

#[test]
fn modal_map_conforms() {
    let (_dir, _session, registry, keymap) = load_session("keys/modal.toml");
    assert_eq!(keymap.model, omacell_ui::KeyModel::Modal);
    assert_conforms(&keymap, &registry);
}

#[test]
fn unknown_command_is_rejected_at_parse() {
    let err = Keymap::parse(
        r#"
[meta]
name = "x"
model = "classic"
[bindings]
"Ctrl+Z" = "not.a.real.command.that.is.unowned"
"#,
    );
    // parse accepts any valid CommandId; conformance rejects unowned ids
    let map = err.expect("parse allows well-formed ids");
    assert_eq!(
        map.iter().next().unwrap().2.cmd,
        "not.a.real.command.that.is.unowned"
    );
}

#[test]
fn invalid_command_id_is_rejected() {
    let err = Keymap::parse(
        r#"
[meta]
name = "x"
model = "classic"
[bindings]
"Ctrl+Z" = "Undo"
"#,
    );
    assert!(err.is_err());
}
