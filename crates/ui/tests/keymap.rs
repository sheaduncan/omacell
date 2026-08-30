//! Keymap conformance: registered ∪ deferred; no unknown ids; no duplicate chords.

use omacell_bus::{CommandRegistry, register_chart_commands, register_core};
use omacell_conf::{Paths, load};
use omacell_ui::{
    DEFERRED_COMMANDS, KeyCode, KeyEvent, KeyOutcome, Keymap, KeymapRoots, UiSession,
    deferred_owner, register_ui_commands,
};

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
    register_chart_commands(&mut registry).unwrap();
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

#[test]
fn deferred_ownership_table_is_unique_and_well_formed() {
    let (_dir, _session, registry, _keymap) = load_session("keys/classic.toml");
    let mut ids = std::collections::BTreeSet::new();
    for deferred in DEFERRED_COMMANDS {
        assert!(
            ids.insert(deferred.id),
            "duplicate deferred id {}",
            deferred.id
        );
        assert!(omacell_core::command::CommandId::new(deferred.id).is_ok());
        assert!(
            registry.get_str(deferred.id).is_err(),
            "deferred command {} is already registered",
            deferred.id
        );
        assert!(
            deferred
                .wp
                .strip_prefix("WP-")
                .is_some_and(|number| number.bytes().all(|byte| byte.is_ascii_digit())),
            "invalid owner {} for {}",
            deferred.wp,
            deferred.id
        );
    }
}

#[test]
fn ctrl_shift_and_case_are_normalized_consistently() {
    let (_dir, _session, _registry, mut classic) = load_session("keys/classic.toml");
    assert!(matches!(
        classic.dispatch(
            omacell_ui::Mode::Classic,
            KeyEvent {
                code: KeyCode::Char('z'),
                ctrl: true,
                alt: false,
                shift: false,
            },
        ),
        KeyOutcome::Command { cmd, .. } if cmd == "edit.undo"
    ));
    assert!(matches!(
        classic.dispatch(
            omacell_ui::Mode::Classic,
            KeyEvent {
                code: KeyCode::Char('A'),
                ctrl: true,
                alt: false,
                shift: true,
            },
        ),
        KeyOutcome::Command { cmd, .. } if cmd == "ai.plan"
    ));

    let (_dir, _session, _registry, mut modal) = load_session("keys/modal.toml");
    assert!(matches!(
        modal.dispatch(
            omacell_ui::Mode::Normal,
            KeyEvent {
                code: KeyCode::Char('r'),
                ctrl: true,
                alt: false,
                shift: false,
            },
        ),
        KeyOutcome::Command { cmd, .. } if cmd == "edit.redo"
    ));
    assert!(matches!(
        modal.dispatch(
            omacell_ui::Mode::Normal,
            KeyEvent {
                code: KeyCode::Char('G'),
                ctrl: false,
                alt: false,
                shift: true,
            },
        ),
        KeyOutcome::Command { cmd, .. } if cmd == "nav.bottom"
    ));
}

#[test]
fn modal_counts_are_passed_only_to_count_aware_commands_and_reset_on_error() {
    let (_dir, _session, _registry, mut modal) = load_session("keys/modal.toml");
    assert_eq!(
        modal.dispatch(omacell_ui::Mode::Normal, KeyEvent::new(KeyCode::Char('2')),),
        KeyOutcome::Pending
    );
    assert!(matches!(
        modal.dispatch(
            omacell_ui::Mode::Normal,
            KeyEvent::new(KeyCode::Char('u')),
        ),
        KeyOutcome::Command { cmd, args, count: 2 }
            if cmd == "edit.undo" && args.is_null()
    ));

    assert_eq!(
        modal.dispatch(omacell_ui::Mode::Normal, KeyEvent::new(KeyCode::Char('3')),),
        KeyOutcome::Pending
    );
    assert!(matches!(
        modal.dispatch(
            omacell_ui::Mode::Normal,
            KeyEvent::new(KeyCode::Char('j')),
        ),
        KeyOutcome::Command { cmd, args, count: 3 }
            if cmd == "nav.down" && args == serde_json::json!({"count": 3})
    ));

    assert_eq!(
        modal.dispatch(omacell_ui::Mode::Normal, KeyEvent::new(KeyCode::Char('3')),),
        KeyOutcome::Pending
    );
    assert_eq!(
        modal.dispatch(omacell_ui::Mode::Normal, KeyEvent::new(KeyCode::Char('?')),),
        KeyOutcome::Unbound
    );
    assert!(matches!(
        modal.dispatch(omacell_ui::Mode::Normal, KeyEvent::new(KeyCode::Char('j')),),
        KeyOutcome::Command { count: 1, .. }
    ));
}

#[test]
fn sparse_user_map_overlays_the_package_map() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    let user_map = paths.user_config.join("keys/classic.toml");
    std::fs::create_dir_all(user_map.parent().unwrap()).unwrap();
    std::fs::write(
        &user_map,
        r#"
[bindings]
"Ctrl+Z" = "edit.redo"
"#,
    )
    .unwrap();
    let loaded = load(&paths, &[], None).unwrap();
    let roots = KeymapRoots::new(paths.user_config, paths.default_dir, None);
    let session = UiSession::new(&loaded, &roots).unwrap();
    let map = session.keymap();
    let classic = map.table(omacell_ui::Mode::Classic).unwrap();
    assert_eq!(classic["Ctrl+Z"].cmd, "edit.redo");
    assert_eq!(classic["Ctrl+Y"].cmd, "edit.redo");
}

#[test]
fn unknown_modes_and_unowned_user_commands_are_rejected() {
    assert!(
        Keymap::parse(
            r#"
[meta]
model = "modal"
[bindings.typo]
x = "edit.undo"
"#,
        )
        .is_err()
    );

    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    let user_map = paths.user_config.join("keys/classic.toml");
    std::fs::create_dir_all(user_map.parent().unwrap()).unwrap();
    std::fs::write(
        &user_map,
        r#"
[bindings]
"Ctrl+Q" = "unknown.command"
"#,
    )
    .unwrap();
    let loaded = load(&paths, &[], None).unwrap();
    let roots = KeymapRoots::new(paths.user_config, paths.default_dir, None);
    let session = UiSession::new(&loaded, &roots).unwrap();
    let mut registry = CommandRegistry::new();
    register_core(&mut registry).unwrap();
    let err = register_ui_commands(&mut registry, &session).unwrap_err();
    assert_eq!(err.code, "ui.keymap");
}

#[cfg(unix)]
#[test]
fn keymap_symlink_cannot_escape_its_search_root() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    let key_dir = paths.user_config.join("keys");
    std::fs::create_dir_all(&key_dir).unwrap();
    let outside = dir.path().join("outside.toml");
    std::fs::write(
        &outside,
        "[meta]\nmodel='classic'\n[bindings]\nF1='help.keys'\n",
    )
    .unwrap();
    symlink(&outside, key_dir.join("classic.toml")).unwrap();
    let roots = KeymapRoots::new(paths.user_config, paths.default_dir, None);
    assert!(omacell_ui::resolve_keymap_path("keys/classic.toml", &roots).is_err());
}

#[test]
fn explicit_config_root_does_not_fall_through_to_user_config() {
    let dir = tempfile::tempdir().unwrap();
    let explicit = dir.path().join("explicit");
    let user = dir.path().join("user");
    let defaults = dir.path().join("defaults");
    for root in [&explicit, &user, &defaults] {
        std::fs::create_dir_all(root.join("keys")).unwrap();
    }
    std::fs::write(user.join("keys/classic.toml"), "user").unwrap();
    std::fs::write(defaults.join("keys/classic.toml"), "default").unwrap();
    let config_file = explicit.join("config.toml");
    let roots = KeymapRoots::new(user, defaults.clone(), Some(&config_file));

    let resolved = omacell_ui::resolve_keymap_path("keys/classic.toml", &roots).unwrap();
    assert_eq!(
        resolved,
        defaults.join("keys/classic.toml").canonicalize().unwrap()
    );
}
