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

#[test]
fn completed_reports_have_only_wp28_human_gates_unchecked() {
    let root = workspace_root();
    let reports = root.join("reports");
    let mut unchecked = Vec::new();
    for entry in fs::read_dir(&reports).expect("read reports") {
        let path = entry.expect("report entry").path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("WP-") || path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("read work-package report");
        for (line_index, line) in text.lines().enumerate() {
            if line.contains("[ ]") {
                unchecked.push((name.to_string(), line_index + 1, line.to_string()));
            }
        }
    }
    assert_eq!(
        unchecked.len(),
        4,
        "unexpected unchecked report items: {unchecked:#?}"
    );
    assert!(
        unchecked.iter().all(|(name, _, line)| {
            matches!(name.as_str(), "WP-15.md" | "WP-S2.md") && line.contains("HUMAN / WP-28 G4")
        }),
        "completed reports may leave unchecked only the four owned WP-28 human gates: {unchecked:#?}"
    );
}

#[test]
fn merged_contract_reports_do_not_retain_pending_merge_gates() {
    let root = workspace_root();
    let mut paths = Vec::new();
    walk(&root.join("reports"), &mut paths);
    paths.push(root.join("docs/contracts.md"));
    let pending = [
        "must not merge until",
        "PR must not merge",
        "requires human approval before merge",
        "approval pending",
    ];
    let mut violations = Vec::new();
    for path in paths {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for (line_index, line) in text.lines().enumerate() {
            if pending.iter().any(|needle| line.contains(needle)) {
                let rel = path.strip_prefix(&root).unwrap_or(&path);
                violations.push(format!("{}:{}: {line}", rel.display(), line_index + 1));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "merged contracts must record their approval instead of retaining a pending gate:\n{}",
        violations.join("\n")
    );
}

#[test]
fn wp28_distribution_assets_are_complete() {
    let root = workspace_root();
    let packaging = root.join("packaging");
    for relative in [
        "PKGBUILD",
        "PKGBUILD-bin",
        "omacell.install",
        "omacell.desktop",
        "omacell.xml",
        "name.env",
    ] {
        assert!(
            packaging.join(relative).is_file(),
            "missing packaging/{relative}"
        );
    }

    let source = fs::read_to_string(packaging.join("PKGBUILD")).expect("read source PKGBUILD");
    for needle in [
        "build()",
        "check()",
        "package()",
        "options=('!lto')",
        "PKGBUILD_SOURCE_URL",
        "PKGBUILD_SOURCE_SHA256",
        "install -d \"${pkgdir}/usr/share/omacell\"",
        "ttf-carlito",
        "ttf-liberation",
        "/usr/share/omacell",
    ] {
        assert!(source.contains(needle), "source PKGBUILD missing {needle}");
    }
    assert!(
        !source.contains("OMACELL_SOURCE_"),
        "build inputs must not collide with the OMACELL_* runtime config namespace"
    );

    let smoke =
        fs::read_to_string(root.join("scripts/arch-package-smoke.sh")).expect("read Arch smoke");
    for needle in ["PKGBUILD_SOURCE_URL", "PKGBUILD_SOURCE_SHA256"] {
        assert!(smoke.contains(needle), "Arch smoke missing {needle}");
    }

    let book = fs::read_to_string(root.join("book.toml")).expect("read mdBook configuration");
    assert!(
        !book.contains("multilingual"),
        "book.toml must remain compatible with mdBook 0.5"
    );
    let binary = fs::read_to_string(packaging.join("PKGBUILD-bin")).expect("read binary PKGBUILD");
    for needle in ["pkgname=omacell-bin", "package()", "x86_64", "aarch64"] {
        assert!(binary.contains(needle), "binary PKGBUILD missing {needle}");
    }

    let desktop =
        fs::read_to_string(packaging.join("omacell.desktop")).expect("read desktop entry");
    for mime in [
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "application/vnd.ms-excel.sheet.macroEnabled.12",
        "application/vnd.ms-excel",
        "text/csv",
        "text/tab-separated-values",
        "application/vnd.oasis.opendocument.spreadsheet",
        "application/x-omacell",
    ] {
        assert!(desktop.contains(mime), "desktop entry missing {mime}");
    }
    let mime = fs::read_to_string(packaging.join("omacell.xml")).expect("read MIME XML");
    assert!(mime.contains("application/x-omacell"));
    assert!(mime.contains("*.omc"));

    for size in [
        "16x16", "24x24", "32x32", "48x48", "64x64", "128x128", "256x256",
    ] {
        assert!(
            packaging
                .join("icons/hicolor")
                .join(size)
                .join("apps/omacell.png")
                .is_file(),
            "missing hicolor {size} application icon"
        );
    }
    for relative in [
        "icons/hicolor/scalable/apps/omacell.svg",
        "icons/hicolor/symbolic/apps/omacell-symbolic.svg",
        "icons/hicolor/scalable/mimetypes/application-x-omacell.svg",
    ] {
        assert!(
            packaging.join(relative).is_file(),
            "missing packaging/{relative}"
        );
    }

    let mut packaging_text = Vec::new();
    walk(&packaging, &mut packaging_text);
    for path in packaging_text {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        assert!(
            !text.contains("TODO(WP-28)"),
            "shipping scaffold remains in {}",
            path.display()
        );
    }
}

#[test]
fn wp28_manual_and_release_automation_are_present() {
    let root = workspace_root();
    for relative in [
        "book.toml",
        "docs/book/SUMMARY.md",
        "docs/book/manual.md",
        "docs/book/configuration.md",
        "docs/book/cli-reference.md",
        "docs/book/lua-api.md",
        "docs/book/ai-privacy.md",
        "docs/book/omarchy.md",
        "docs/book/parser-limits.md",
        "docs/book/pdf-printing.md",
        "scripts/generate-docs.py",
        "scripts/extract-i18n.py",
        "scripts/check-perf-baselines.py",
        "scripts/rename.sh",
        "scripts/arch-package-smoke.sh",
        ".github/workflows/packaging.yml",
        ".github/workflows/omarchy.yml",
        ".github/workflows/performance.yml",
        ".github/workflows/release.yml",
        "CHANGELOG.md",
        "i18n/en-US/omacell.ftl",
    ] {
        assert!(root.join(relative).is_file(), "missing {relative}");
    }

    let summary = fs::read_to_string(root.join("docs/book/SUMMARY.md")).expect("read SUMMARY");
    for chapter in [
        "manual.md",
        "configuration.md",
        "cli-reference.md",
        "lua-api.md",
        "ai-privacy.md",
        "omarchy.md",
        "parser-limits.md",
        "pdf-printing.md",
    ] {
        assert!(
            summary.contains(chapter),
            "mdBook summary missing {chapter}"
        );
    }

    let rename = fs::read_to_string(root.join("scripts/rename.sh")).expect("read rename script");
    assert!(rename.contains("packaging/name.env"));
    assert!(rename.contains("crates/core/src/product.rs"));
    let nightly =
        fs::read_to_string(root.join(".github/workflows/nightly.yml")).expect("read nightly");
    assert!(nightly.contains("cargo +nightly fuzz list"));
    assert!(nightly.contains("cargo deny check"));
    let release =
        fs::read_to_string(root.join(".github/workflows/release.yml")).expect("read release");
    assert!(release.contains("install -d \"$root/share/omacell\""));
}
