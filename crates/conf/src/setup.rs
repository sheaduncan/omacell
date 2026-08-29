//! `omacell setup omarchy` filesystem actions (spec §7.5–§7.6). Never writes
//! under `/usr/share/omarchy`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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
    chmod_exec(&hook)?;
    report.written.push(hook);

    if opts.link_skill {
        let src = paths.default_dir.join("agents/skills/omacell");
        if src.join("SKILL.md").is_file() {
            for relative in [
                ".agents/skills/omacell",
                ".claude/skills/omacell",
                ".codex/skills/omacell",
                ".pi/agent/skills/omacell",
                ".gemini/config/skills/omacell",
            ] {
                let dest = paths.home.join(relative);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| error::io(e.to_string()))?;
                }
                match std::fs::symlink_metadata(&dest) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        if std::fs::read_link(&dest).ok().as_deref() == Some(src.as_path()) {
                            continue;
                        }
                        report
                            .skipped
                            .push(format!("{} (existing symlink)", dest.display()));
                        continue;
                    }
                    Ok(_) => {
                        report
                            .skipped
                            .push(format!("{} (existing user path)", dest.display()));
                        continue;
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(err) => return Err(error::io(err.to_string())),
                }
                std::os::unix::fs::symlink(&src, &dest).map_err(|e| error::io(e.to_string()))?;
                report.written.push(dest);
            }
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
        if merge_menu_rows(&menu)? {
            report.written.push(menu);
        }
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
    atomic_write(path, body.as_bytes())
}

fn chmod_exec(path: &Path) -> Result<(), omacell_core::error::CoreError> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path).map_err(|e| error::io(e.to_string()))?;
    let mut permissions = meta.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).map_err(|e| error::io(e.to_string()))
}

fn merge_menu_rows(path: &Path) -> Result<bool, omacell_core::error::CoreError> {
    if !path.is_file() {
        write_file(path, MENU_JSONC)?;
        return Ok(true);
    }
    let original = std::fs::read_to_string(path).map_err(|e| error::io(e.to_string()))?;
    let mut missing = Vec::new();
    if !original.contains("\"label\": \"Spreadsheet\"") {
        missing.push(r#"{ "label": "Spreadsheet", "command": "omacell" }"#);
    }
    if !original.contains("\"label\": \"New from clipboard\"") {
        missing.push(r#"{ "label": "New from clipboard", "command": "omacell --clipboard" }"#);
    }
    if missing.is_empty() {
        return Ok(false);
    }

    let rows = original
        .find("\"rows\"")
        .ok_or_else(|| error::schema("existing omarchy-menu.jsonc has no rows array"))?;
    let start = original[rows..]
        .find('[')
        .map(|offset| rows + offset)
        .ok_or_else(|| error::schema("existing omarchy-menu.jsonc rows is not an array"))?;
    let end = matching_array_end(&original, start)
        .ok_or_else(|| error::schema("existing omarchy-menu.jsonc rows array is malformed"))?;
    let inner = &original[start + 1..end];
    let trimmed_len = inner.trim_end().len();
    let insertion = start + 1 + trimmed_len;
    let has_rows = inner[..trimmed_len].contains('{');
    let has_trailing_comma = inner[..trimmed_len].trim_end().ends_with(',');
    let separator = if !has_rows || has_trailing_comma {
        "\n    "
    } else {
        ",\n    "
    };
    let addition = format!("{separator}{}", missing.join(",\n    "));
    let mut merged = String::with_capacity(original.len() + addition.len());
    merged.push_str(&original[..insertion]);
    merged.push_str(&addition);
    merged.push_str(&original[insertion..]);
    atomic_write(path, merged.as_bytes())?;
    Ok(true)
}

fn matching_array_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut line_comment = false;
    let mut block_comment = false;
    let mut index = start;
    while index < bytes.len() {
        let byte = bytes[index];
        let next = bytes.get(index + 1).copied();
        if line_comment {
            if byte == b'\n' {
                line_comment = false;
            }
        } else if block_comment {
            if byte == b'*' && next == Some(b'/') {
                block_comment = false;
                index += 1;
            }
        } else if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
        } else if byte == b'/' && next == Some(b'/') {
            line_comment = true;
            index += 1;
        } else if byte == b'/' && next == Some(b'*') {
            block_comment = true;
            index += 1;
        } else if byte == b'[' {
            depth += 1;
        } else if byte == b']' {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(index);
            }
        }
        index += 1;
    }
    None
}

fn atomic_write(path: &Path, body: &[u8]) -> Result<(), omacell_core::error::CoreError> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .ok_or_else(|| error::io("setup destination has no parent"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| error::io("setup destination has an invalid file name"))?;
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(
        ".{name}.omacell-new-{}-{serial}",
        std::process::id()
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(body)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|e| error::io(e.to_string()))
}
