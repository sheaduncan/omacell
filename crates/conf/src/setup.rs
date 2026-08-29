//! `omacell setup omarchy` filesystem actions (spec §7.5–§7.6). Never writes
//! under `/usr/share/omarchy`.

use std::path::{Path, PathBuf};

use crate::error;
use crate::paths::Paths;
use crate::theme::TEMPLATE;

/// Options for setup.
#[derive(Clone, Debug)]
pub struct SetupOptions {
    /// Write `omarchy-menu.jsonc` rows.
    pub confirm_menu: bool,
    /// Create skill symlinks when the source file exists (WP-21).
    pub link_skill: bool,
}

impl Default for SetupOptions {
    fn default() -> Self {
        Self {
            confirm_menu: false,
            link_skill: true,
        }
    }
}

/// Paths written by a setup run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetupReport {
    /// Created or replaced files.
    pub written: Vec<PathBuf>,
    /// Skipped optional steps.
    pub skipped: Vec<String>,
}

/// Hyprland snippet printed by `--show-hyprland`.
pub const HYPRLAND_SNIPPET: &str = r#"-- ~/.config/hypr/bindings.lua  (pick any chord that is free on your machine)
o.bind("SUPER + ALT + X", "Spreadsheet", "omacell")
"#;

/// Theme-set hook body.
pub const THEME_HOOK: &str = r#"#!/bin/sh
# Omarchy runs this after a theme change; $1 is the theme name.
exec omacell ipc theme.reload --all --quiet
"#;

/// Install template, hook, optional menu and skill links into `paths.home`.
pub fn setup_omarchy(
    paths: &Paths,
    opts: SetupOptions,
) -> Result<SetupReport, omacell_core::error::CoreError> {
    let mut report = SetupReport::default();
    let themed = paths.omarchy_config.join("themed");
    std::fs::create_dir_all(&themed).map_err(|e| error::io(e.to_string()))?;
    let tpl = themed.join("omacell.toml.tpl");
    write_file(&tpl, TEMPLATE)?;
    report.written.push(tpl);

    let hook_dir = paths.omarchy_config.join("hooks/theme-set.d");
    std::fs::create_dir_all(&hook_dir).map_err(|e| error::io(e.to_string()))?;
    let hook = hook_dir.join("omacell");
    write_file(&hook, THEME_HOOK)?;
    chmod_exec(&hook);
    report.written.push(hook);

    if opts.link_skill {
        let src = paths.default_dir.join("agents/skills/omacell/SKILL.md");
        if src.is_file() {
            let dest_dir = paths.home.join(".config/omacell/agents/skills/omacell");
            std::fs::create_dir_all(&dest_dir).map_err(|e| error::io(e.to_string()))?;
            let dest = dest_dir.join("SKILL.md");
            let _ = std::fs::remove_file(&dest);
            std::os::unix::fs::symlink(&src, &dest).map_err(|e| error::io(e.to_string()))?;
            report.written.push(dest);
        } else {
            report
                .skipped
                .push("skill symlink (WP-21 file not shipped yet)".into());
        }
    }

    if opts.confirm_menu {
        let ext = paths.omarchy_config.join("extensions");
        std::fs::create_dir_all(&ext).map_err(|e| error::io(e.to_string()))?;
        let menu = ext.join("omarchy-menu.jsonc");
        write_file(&menu, MENU_JSONC)?;
        report.written.push(menu);
    } else {
        report
            .skipped
            .push("omarchy-menu.jsonc (not confirmed)".into());
    }
    Ok(report)
}

const MENU_JSONC: &str = r#"{
  "rows": [
    { "label": "Spreadsheet", "command": "omacell" },
    { "label": "New from clipboard", "command": "omacell --clipboard" }
  ]
}
"#;

fn write_file(path: &Path, body: &str) -> Result<(), omacell_core::error::CoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| error::io(e.to_string()))?;
    }
    std::fs::write(path, body).map_err(|e| error::io(e.to_string()))
}

fn chmod_exec(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut p = meta.permissions();
        p.set_mode(0o755);
        let _ = std::fs::set_permissions(path, p);
    }
}
