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

/// Paths removed or deliberately retained by an uninstall run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UninstallReport {
    /// Unchanged Omacell-owned files and links that were removed.
    pub removed: Vec<PathBuf>,
    /// Missing or user-modified assets that were retained.
    pub skipped: Vec<String>,
}

/// Hyprland snippet printed by `--show-hyprland`.
pub const HYPRLAND_SNIPPET: &str = r#"-- ~/.config/hypr/bindings.lua  (pick any chord that is free on your machine)
o.bind("SUPER + ALT + X", "Spreadsheet", { launch = "omacell" })
"#;

/// Theme-set hook body.
pub const THEME_HOOK: &str = r#"#!/bin/sh
# Omarchy runs this after a theme change; $1 is the theme name.
omacell ipc theme.reload --all --quiet || :
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
                ".config/crush/skills/omacell",
                ".config/opencode/skills/omacell",
                ".copilot/skills/omacell",
                ".gemini/config/skills/omacell",
                ".grok/skills/omacell",
                ".pi/agent/skills/omacell",
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
    if !menu_has_command(&original, "omacell")? {
        missing.push(r#"{ "label": "Spreadsheet", "command": "omacell" }"#);
    }
    if !menu_has_command(&original, "omacell --clipboard")? {
        missing.push(r#"{ "label": "New from clipboard", "command": "omacell --clipboard" }"#);
    }
    if missing.is_empty() {
        return Ok(false);
    }

    let (start, end) = menu_array_bounds(&original)?;
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

/// Remove unchanged files, matching skill links, and optionally Omacell menu
/// rows installed by [`setup_omarchy`]. User-modified files and directories are
/// retained.
pub fn uninstall_omarchy(
    paths: &Paths,
    remove_menu: bool,
) -> Result<UninstallReport, omacell_core::error::CoreError> {
    let mut report = UninstallReport::default();
    remove_owned_file(
        &paths.omarchy_config.join("themed/omacell.toml.tpl"),
        TEMPLATE.as_bytes(),
        &mut report,
    )?;
    remove_owned_file(
        &paths.omarchy_config.join("hooks/theme-set.d/omacell"),
        THEME_HOOK.as_bytes(),
        &mut report,
    )?;

    let skill = paths.default_dir.join("agents/skills/omacell");
    for relative in [
        ".agents/skills/omacell",
        ".claude/skills/omacell",
        ".codex/skills/omacell",
        ".config/crush/skills/omacell",
        ".config/opencode/skills/omacell",
        ".copilot/skills/omacell",
        ".gemini/config/skills/omacell",
        ".grok/skills/omacell",
        ".pi/agent/skills/omacell",
    ] {
        let path = paths.home.join(relative);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.file_type().is_symlink()
                    && std::fs::read_link(&path).ok().as_deref() == Some(skill.as_path()) =>
            {
                std::fs::remove_file(&path).map_err(|error| crate::error::io(error.to_string()))?;
                report.removed.push(path);
            }
            Ok(_) => report
                .skipped
                .push(format!("{} (not an Omacell skill link)", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(crate::error::io(error.to_string())),
        }
    }

    if remove_menu {
        let path = paths.omarchy_config.join("extensions/omarchy-menu.jsonc");
        if path.is_file() {
            let original = std::fs::read_to_string(&path)
                .map_err(|error| crate::error::io(error.to_string()))?;
            let updated = remove_menu_commands(&original, &["omacell", "omacell --clipboard"])?;
            if updated != original {
                atomic_write(&path, updated.as_bytes())?;
                report.removed.push(path);
            }
        }
    }
    Ok(report)
}

fn remove_owned_file(
    path: &Path,
    expected: &[u8],
    report: &mut UninstallReport,
) -> Result<(), omacell_core::error::CoreError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(crate::error::io(error.to_string())),
    };
    if !metadata.is_file() {
        report
            .skipped
            .push(format!("{} (not an owned regular file)", path.display()));
        return Ok(());
    }
    let actual = std::fs::read(path).map_err(|error| crate::error::io(error.to_string()))?;
    if actual != expected {
        report
            .skipped
            .push(format!("{} (modified by user)", path.display()));
        return Ok(());
    }
    std::fs::remove_file(path).map_err(|error| crate::error::io(error.to_string()))?;
    report.removed.push(path.to_path_buf());
    Ok(())
}

fn menu_has_command(text: &str, expected: &str) -> Result<bool, omacell_core::error::CoreError> {
    Ok(menu_row_spans(text)?
        .into_iter()
        .any(|(start, end)| menu_row_command(&text[start..end]).as_deref() == Some(expected)))
}

fn remove_menu_commands(
    text: &str,
    commands: &[&str],
) -> Result<String, omacell_core::error::CoreError> {
    let mut output = text.to_string();
    for (start, end) in menu_row_spans(text)?.into_iter().rev() {
        let Some(command) = menu_row_command(&text[start..end]) else {
            continue;
        };
        if !commands.contains(&command.as_str()) {
            continue;
        }
        let mut remove_start = start;
        let mut remove_end = end;
        let after = output[remove_end..]
            .char_indices()
            .find(|(_, character)| !character.is_whitespace())
            .map(|(offset, _)| remove_end + offset);
        if let Some(index) = after
            && output.as_bytes().get(index) == Some(&b',')
        {
            remove_end = index + 1;
        } else {
            let before = output[..remove_start]
                .char_indices()
                .rev()
                .find(|(_, character)| !character.is_whitespace())
                .map(|(index, _)| index);
            if let Some(index) = before
                && output.as_bytes().get(index) == Some(&b',')
            {
                remove_start = index;
            }
        }
        output.replace_range(remove_start..remove_end, "");
    }
    Ok(output)
}

fn menu_row_spans(text: &str) -> Result<Vec<(usize, usize)>, omacell_core::error::CoreError> {
    let (start, end) = menu_array_bounds(text)?;
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut index = start + 1;
    let mut in_string = false;
    let mut escaped = false;
    let mut line_comment = false;
    let mut block_comment = false;
    while index < end {
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
        } else if byte == b'{' {
            let object_end = matching_object_end(text, index).ok_or_else(|| {
                error::schema("existing omarchy-menu.jsonc row object is malformed")
            })?;
            spans.push((index, object_end + 1));
            index = object_end;
        }
        index += 1;
    }
    Ok(spans)
}

fn menu_array_bounds(text: &str) -> Result<(usize, usize), omacell_core::error::CoreError> {
    let without_comments = strip_jsonc_comments(text);
    let bytes = without_comments.as_bytes();
    let mut search_from = 0usize;
    while let Some(relative) = without_comments[search_from..].find("\"rows\"") {
        let key_start = search_from + relative;
        let mut index = key_start + "\"rows\"".len();
        while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
            index += 1;
        }
        if bytes.get(index) != Some(&b':') {
            search_from = index;
            continue;
        }
        index += 1;
        while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
            index += 1;
        }
        if bytes.get(index) != Some(&b'[') {
            return Err(error::schema(
                "existing omarchy-menu.jsonc rows is not an array",
            ));
        }
        let end = matching_array_end(text, index)
            .ok_or_else(|| error::schema("existing omarchy-menu.jsonc rows array is malformed"))?;
        return Ok((index, end));
    }
    Err(error::schema(
        "existing omarchy-menu.jsonc has no rows array",
    ))
}

fn menu_row_command(row: &str) -> Option<String> {
    let json = strip_jsonc_comments(row);
    serde_json::from_str::<serde_json::Value>(&json)
        .ok()?
        .get("command")?
        .as_str()
        .map(str::to_string)
}

fn strip_jsonc_comments(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut line_comment = false;
    let mut block_comment = false;
    while index < bytes.len() {
        let byte = bytes[index];
        let next = bytes.get(index + 1).copied();
        if line_comment {
            if byte == b'\n' {
                line_comment = false;
                output.push(byte);
            } else {
                output.push(b' ');
            }
        } else if block_comment {
            if byte == b'*' && next == Some(b'/') {
                output.extend_from_slice(b"  ");
                block_comment = false;
                index += 1;
            } else if byte == b'\n' {
                output.push(byte);
            } else {
                output.push(b' ');
            }
        } else if in_string {
            output.push(byte);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
            output.push(byte);
        } else if byte == b'/' && next == Some(b'/') {
            output.extend_from_slice(b"  ");
            line_comment = true;
            index += 1;
        } else if byte == b'/' && next == Some(b'*') {
            output.extend_from_slice(b"  ");
            block_comment = true;
            index += 1;
        } else {
            output.push(byte);
        }
        index += 1;
    }
    String::from_utf8(output).unwrap_or_default()
}

fn matching_object_end(text: &str, start: usize) -> Option<usize> {
    matching_delimiter_end(text, start, b'{', b'}')
}

fn matching_array_end(text: &str, start: usize) -> Option<usize> {
    matching_delimiter_end(text, start, b'[', b']')
}

fn matching_delimiter_end(text: &str, start: usize, open: u8, close: u8) -> Option<usize> {
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
        } else if byte == open {
            depth += 1;
        } else if byte == close {
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
    let (target, existing_permissions) = match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let target =
                std::fs::canonicalize(path).map_err(|error| error::io(error.to_string()))?;
            let target_metadata =
                std::fs::metadata(&target).map_err(|error| error::io(error.to_string()))?;
            if !target_metadata.is_file() {
                return Err(error::io("setup destination is not a regular file"));
            }
            (target, Some(target_metadata.permissions()))
        }
        Ok(metadata) if metadata.is_file() => (path.to_path_buf(), Some(metadata.permissions())),
        Ok(_) => return Err(error::io("setup destination is not a regular file")),
        Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => {
            (path.to_path_buf(), None)
        }
        Err(problem) => return Err(error::io(problem.to_string())),
    };
    let parent = target
        .parent()
        .ok_or_else(|| error::io("setup destination has no parent"))?;
    let name = target
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
        if let Some(permissions) = existing_permissions {
            std::fs::set_permissions(&temporary, permissions)?;
        }
        std::fs::rename(&temporary, &target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|e| error::io(e.to_string()))
}
