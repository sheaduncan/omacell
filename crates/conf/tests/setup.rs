//! `setup omarchy` writes only under the fake $HOME.

use omacell_conf::paths::Paths;
use omacell_conf::setup::{HYPRLAND_SNIPPET, SetupOptions, setup_omarchy};

const SKILL_LINKS: &[&str] = &[
    ".agents/skills/omacell",
    ".claude/skills/omacell",
    ".codex/skills/omacell",
    ".config/crush/skills/omacell",
    ".config/opencode/skills/omacell",
    ".copilot/skills/omacell",
    ".gemini/config/skills/omacell",
    ".grok/skills/omacell",
    ".pi/agent/skills/omacell",
];

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

    for relative in SKILL_LINKS {
        assert!(
            std::fs::symlink_metadata(dir.path().join(relative))
                .unwrap()
                .file_type()
                .is_symlink(),
            "missing skill link {relative}"
        );
    }

    let src = paths
        .default_dir
        .join("agents/skills/omacell")
        .canonicalize()
        .unwrap();
    for relative in SKILL_LINKS {
        let dest = dir.path().join(relative);
        let target = std::fs::read_link(&dest).unwrap();
        let resolved = dest.parent().unwrap().join(&target).canonicalize().unwrap();
        assert_eq!(resolved, src, "{relative}");
        assert!(resolved.join("SKILL.md").is_file());
    }

    let again = setup_omarchy(
        &paths,
        SetupOptions {
            confirm_menu: false,
            link_skill: true,
        },
    )
    .unwrap();
    assert!(
        again
            .written
            .iter()
            .all(|p| !p.to_string_lossy().contains("skills/omacell")),
        "skill links must be idempotent: {again:?}"
    );

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
            ".config/crush",
            ".config/crush/skills",
            ".config/crush/skills/omacell",
            ".config/omarchy",
            ".config/omarchy/hooks",
            ".config/omarchy/hooks/theme-set.d",
            ".config/omarchy/hooks/theme-set.d/omacell",
            ".config/omarchy/themed",
            ".config/omarchy/themed/omacell.toml.tpl",
            ".config/opencode",
            ".config/opencode/skills",
            ".config/opencode/skills/omacell",
            ".copilot",
            ".copilot/skills",
            ".copilot/skills/omacell",
            ".gemini",
            ".gemini/config",
            ".gemini/config/skills",
            ".gemini/config/skills/omacell",
            ".grok",
            ".grok/skills",
            ".grok/skills/omacell",
            ".pi",
            ".pi/agent",
            ".pi/agent/skills",
            ".pi/agent/skills/omacell",
        ]
    );
}

#[test]
fn hyprland_snippet_uses_the_quattro_launch_table() {
    assert!(
        HYPRLAND_SNIPPET.contains(r#"{ launch = "omacell" }"#),
        "{HYPRLAND_SNIPPET}"
    );
    assert!(!HYPRLAND_SNIPPET.contains(r#", "omacell")"#));
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
