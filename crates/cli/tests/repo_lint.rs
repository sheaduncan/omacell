//! Fails if a work-package TODO marker lacks a `WP-` reference.
use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/cli -> workspace root")
        .to_path_buf()
}

fn skip_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | "target" | ".direnv" | "node_modules" | ".idea" | ".vscode"
    )
}

fn is_probably_text(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => !matches!(
            ext,
            "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "webp"
                | "ico"
                | "pdf"
                | "zip"
                | "gz"
                | "tgz"
                | "xz"
                | "zst"
                | "bin"
                | "o"
                | "so"
                | "a"
                | "rlib"
                | "rmeta"
                | "woff"
                | "woff2"
                | "ttf"
                | "otf"
                | "lock"
        ),
        None => matches!(
            path.file_name().and_then(|n| n.to_str()),
            Some(
                "justfile"
                    | "LICENSE"
                    | "Cargo.toml"
                    | "deny.toml"
                    | "rustfmt.toml"
                    | "clippy.toml"
                    | ".gitignore"
            )
        ),
    }
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if skip_dir(entry.file_name().to_string_lossy().as_ref()) {
                continue;
            }
            walk(&path, files);
        } else if is_probably_text(&path) {
            files.push(path);
        }
    }
}

/// Bare `TODO` with a parenthesized id looks like a marker; a search string does not.
fn looks_like_marker_inner(inner: &str) -> bool {
    let trimmed = inner.trim();
    !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ',' | ' ' | '/'))
}

#[test]
fn todo_markers_name_a_work_package() {
    let root = workspace_root();
    let mut files = Vec::new();
    walk(&root, &mut files);
    assert!(
        !files.is_empty(),
        "repo-lint found no files under {}",
        root.display()
    );

    let needle = format!("{}{}", "TODO", "(");
    let mut violations = Vec::new();
    for path in &files {
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        for (idx, _) in text.match_indices(&needle) {
            let after = &text[idx + needle.len()..];
            let Some(end) = after.find(')') else {
                continue;
            };
            let inner = &after[..end];
            if looks_like_marker_inner(inner) && !inner.contains("WP-") {
                let rel = path.strip_prefix(&root).unwrap_or(path);
                violations.push(format!("{}: {needle}{inner})", rel.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "TODO markers must name a work package, e.g. TODO(WP-12):\n{}",
        violations.join("\n")
    );
}

fn assert_copies_match(canonical: &Path, copy: &Path) {
    let left = fs::read_to_string(canonical)
        .unwrap_or_else(|err| panic!("read {}: {err}", canonical.display()));
    let right =
        fs::read_to_string(copy).unwrap_or_else(|err| panic!("read {}: {err}", copy.display()));
    assert_eq!(
        left,
        right,
        "{} and {} drifted; edit the canonical path and copy it",
        canonical.display(),
        copy.display()
    );
}

#[test]
fn agents_md_bundle_copy_matches_root() {
    let root = workspace_root();
    assert_copies_match(&root.join("AGENTS.md"), &root.join("docs/build/AGENTS.md"));
}

#[test]
fn design_spec_bundle_copy_matches_docs_spec() {
    let root = workspace_root();
    assert_copies_match(
        &root.join("docs/spec/omacell-design-spec.md"),
        &root.join("docs/build/spec/omacell-design-spec.md"),
    );
}

fn underscore_command_candidate(token: &str) -> bool {
    let Some((namespace, command)) = token.split_once('.') else {
        return false;
    };
    if namespace.is_empty()
        || command.is_empty()
        || command.contains('.')
        || !namespace.contains('_')
        || !namespace
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        || !command.bytes().all(|byte| byte.is_ascii_lowercase())
    {
        return false;
    }
    // These exact two-segment shapes are documentation filenames, not command
    // ids. Keep the exemption about the suffix rather than individual files.
    !matches!(
        command,
        "csv"
            | "html"
            | "json"
            | "lua"
            | "md"
            | "ods"
            | "pdf"
            | "png"
            | "py"
            | "rs"
            | "svg"
            | "toml"
            | "tsv"
            | "txt"
            | "xls"
            | "xlsx"
            | "xml"
            | "yaml"
            | "yml"
    )
}

#[test]
fn docs_do_not_spell_command_ids_with_underscores() {
    assert!(underscore_command_candidate("file_open.run"));
    assert!(!underscore_command_candidate("file.open"));
    assert!(!underscore_command_candidate("date_system.rs"));

    let root = workspace_root();
    let docs = root.join("docs");
    let mut files = Vec::new();
    walk(&docs, &mut files);
    let mut violations = Vec::new();
    for path in &files {
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        for (line_index, line) in text.lines().enumerate() {
            for token in line.split(|character: char| {
                !(character.is_ascii_lowercase() || matches!(character, '_' | '.'))
            }) {
                if underscore_command_candidate(token) {
                    let rel = path.strip_prefix(&root).unwrap_or(path);
                    violations.push(format!("{}:{}: {token}", rel.display(), line_index + 1));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "command ids use lowercase dotted segments, never underscores:\n{}",
        violations.join("\n")
    );
}
