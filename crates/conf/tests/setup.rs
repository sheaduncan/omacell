//! `setup omarchy` writes only under the fake $HOME.

use omacell_conf::paths::Paths;
use omacell_conf::setup::{SetupOptions, setup_omarchy};

fn relative_entries(root: &std::path::Path) -> Vec<String> {
    fn walk(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            out.push(
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            );
            if entry.file_type().unwrap().is_dir() {
                walk(root, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

#[test]
fn setup_without_menu_writes_only_expected_files() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    let report = setup_omarchy(
        &paths,
        SetupOptions {
            confirm_menu: false,
            link_skill: true,
        },
    )
    .unwrap();
    let tpl = paths.omarchy_config.join("themed/omacell.toml.tpl");
    let hook = paths.omarchy_config.join("hooks/theme-set.d/omacell");
    assert!(tpl.is_file(), "{report:?}");
    assert!(hook.is_file());
    assert!(
        !paths
            .omarchy_config
            .join("extensions/omarchy-menu.jsonc")
            .exists()
    );
    assert!(report.skipped.iter().any(|s| s.contains("menu")));
    assert!(report.written.iter().all(|p| p.starts_with(dir.path())));

    for relative in [
        ".agents/skills/omacell",
        ".claude/skills/omacell",
        ".codex/skills/omacell",
        ".pi/agent/skills/omacell",
        ".gemini/config/skills/omacell",
    ] {
        assert!(
            std::fs::symlink_metadata(dir.path().join(relative))
                .unwrap()
                .file_type()
                .is_symlink(),
            "missing skill link {relative}"
        );
    }

    assert_eq!(
        relative_entries(dir.path()),
        vec![
            ".agents",
            ".agents/skills",
            ".agents/skills/omacell",
            ".claude",
            ".claude/skills",
            ".claude/skills/omacell",
            ".codex",
            ".codex/skills",
            ".codex/skills/omacell",
            ".config",
            ".config/omarchy",
            ".config/omarchy/hooks",
            ".config/omarchy/hooks/theme-set.d",
            ".config/omarchy/hooks/theme-set.d/omacell",
            ".config/omarchy/themed",
            ".config/omarchy/themed/omacell.toml.tpl",
            ".gemini",
            ".gemini/config",
            ".gemini/config/skills",
            ".gemini/config/skills/omacell",
            ".pi",
            ".pi/agent",
            ".pi/agent/skills",
            ".pi/agent/skills/omacell",
        ]
    );
}

#[test]
fn setup_with_menu_confirmation_writes_jsonc() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    setup_omarchy(
        &paths,
        SetupOptions {
            confirm_menu: true,
            link_skill: false,
        },
    )
    .unwrap();
    assert!(
        paths
            .omarchy_config
            .join("extensions/omarchy-menu.jsonc")
            .is_file()
    );
}

#[test]
fn setup_preserves_existing_menu_rows_and_comments() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    let menu = paths.omarchy_config.join("extensions/omarchy-menu.jsonc");
    std::fs::create_dir_all(menu.parent().unwrap()).unwrap();
    std::fs::write(
        &menu,
        "{\n  // keep this comment\n  \"rows\": [\n    { \"label\": \"Terminal\", \"command\": \"foot\" }\n  ]\n}\n",
    )
    .unwrap();

    setup_omarchy(
        &paths,
        SetupOptions {
            confirm_menu: true,
            link_skill: false,
        },
    )
    .unwrap();

    let merged = std::fs::read_to_string(&menu).unwrap();
    assert!(merged.contains("keep this comment"), "{merged}");
    assert!(merged.contains("Terminal"), "{merged}");
    assert!(merged.contains("Spreadsheet"), "{merged}");
    assert!(merged.contains("New from clipboard"), "{merged}");

    setup_omarchy(
        &paths,
        SetupOptions {
            confirm_menu: true,
            link_skill: false,
        },
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(&menu)
            .unwrap()
            .matches("New from clipboard")
            .count(),
        1,
        "setup must be idempotent"
    );
}

#[test]
fn malformed_existing_menu_is_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    let menu = paths.omarchy_config.join("extensions/omarchy-menu.jsonc");
    std::fs::create_dir_all(menu.parent().unwrap()).unwrap();
    let original = "{ \"other\": true }\n";
    std::fs::write(&menu, original).unwrap();

    assert!(
        setup_omarchy(
            &paths,
            SetupOptions {
                confirm_menu: true,
                link_skill: false,
            },
        )
        .is_err()
    );
    assert_eq!(std::fs::read_to_string(menu).unwrap(), original);
}
