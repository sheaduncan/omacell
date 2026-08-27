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
