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
        "options=('!lto' 'docs')",
        "PKGBUILD_SOURCE_URL",
        "PKGBUILD_SOURCE_SHA256",
        "export CARGO_TARGET_DIR=\"${srcdir}/${pkgname}-${pkgver}/target\"",
        "install -d \"${pkgdir}/usr/share/omacell\"",
        "${pkgdir}/usr/lib/omacell/omacell-xls-worker",
        "cp -a book/. \"${pkgdir}/usr/share/doc/${pkgname}/manual/\"",
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
    assert!(
        !source.contains("book/html"),
        "source packaging must use book.toml's configured build directory"
    );

    let smoke =
        fs::read_to_string(root.join("scripts/arch-package-smoke.sh")).expect("read Arch smoke");
    for needle in [
        "PKGBUILD_SOURCE_URL",
        "PKGBUILD_SOURCE_SHA256",
        "smoke: manual in package archive",
    ] {
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
    assert!(release.contains("$root/lib/omacell/omacell-xls-worker"));
    assert!(release.contains("cp -a book/. \"$root/share/doc/omacell/manual/\""));
    assert!(!release.contains("book/html"));
}

#[test]
fn wp28_omarchy_packaging_and_performance_lanes_do_not_drift() {
    let root = workspace_root();
    let omarchy = fs::read_to_string(root.join(".github/workflows/omarchy.yml"))
        .expect("read Omarchy workflow");
    for input in ["PKGBUILD_SOURCE_URL", "PKGBUILD_SOURCE_SHA256"] {
        assert!(
            omarchy.contains(input),
            "Omarchy workflow must pass the source recipe's {input} input"
        );
    }
    assert!(
        !omarchy.contains("OMACELL_SOURCE_"),
        "build inputs must not use the OMACELL_* runtime namespace"
    );

    let reload =
        fs::read_to_string(root.join("crates/gui/tests/reload.rs")).expect("read GUI reload tests");
    assert!(
        !reload.contains("theme reload took"),
        "shared/debug tests must not enforce the theme-reload wall clock"
    );
    let performance = fs::read_to_string(root.join(".github/workflows/performance.yml"))
        .expect("read performance workflow");
    assert!(
        performance.contains("cargo bench -p omacell-conf --bench theme_reload"),
        "fixed-host performance workflow must retain the theme-reload benchmark"
    );
}

#[test]
fn wp28_release_handoffs_are_implemented_and_reconciled() {
    let root = workspace_root();
    let setup = fs::read_to_string(root.join("crates/conf/src/setup.rs")).expect("read setup");
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
        assert!(setup.contains(relative), "setup omarchy missing {relative}");
    }
    assert!(setup.contains(r#"{ launch = "omacell" }"#));

    let charts = fs::read_to_string(root.join("crates/bus/src/chart.rs")).expect("read charts");
    for command in [
        "chart.axistitle",
        "chart.move",
        "chart.resize",
        "chart.title",
    ] {
        assert!(
            charts.contains(command),
            "chart release command missing {command}"
        );
    }

    for (relative, stale) in [
        (
            "reports/WP-16.md",
            "Per-cell typeface substitution and an explicit shaping cache are not implemented",
        ),
        ("reports/WP-25.md", "WP-28 release surface"),
        ("reports/WP-26.md", "WP-28 release closure"),
        (
            "docs/open-question-triage-2026-08-31.md",
            "split this into WP-28a",
        ),
        (
            "reports/integration-audit-2026-09-01.md",
            "minimal release editing is WP-28",
        ),
        (
            "reports/integration-audit-2026-09-01.md",
            "Print gaps are explicitly owned by WP-28",
        ),
    ] {
        let text = fs::read_to_string(root.join(relative)).expect("read reconciled report");
        assert!(
            !text.contains(stale),
            "stale handoff in {relative}: {stale}"
        );
    }
}

fn workflow_job_count(workflow: &str) -> usize {
    let mut in_jobs = false;
    let mut count = 0;
    for line in workflow.lines() {
        if line == "jobs:" {
            in_jobs = true;
            continue;
        }
        if in_jobs && !line.is_empty() && !line.starts_with(' ') {
            break;
        }
        let Some(rest) = line.strip_prefix("  ") else {
            continue;
        };
        if in_jobs && !rest.starts_with(' ') && rest.ends_with(':') {
            count += 1;
        }
    }
    count
}

#[test]
fn wp28_workflows_are_bounded_and_fail_closed() {
    let root = workspace_root();
    let workflows = root.join(".github/workflows");
    for name in [
        "ci.yml",
        "nightly.yml",
        "omarchy.yml",
        "packaging.yml",
        "performance.yml",
        "release.yml",
    ] {
        let text = fs::read_to_string(workflows.join(name)).expect("read workflow");
        let jobs = workflow_job_count(&text);
        assert!(jobs > 0, "{name} declares no jobs");
        assert_eq!(
            text.matches("timeout-minutes:").count(),
            jobs,
            "every {name} job needs an explicit timeout"
        );
    }

    let justfile = fs::read_to_string(root.join("justfile")).expect("read justfile");
    assert!(
        justfile.contains("RUSTDOCFLAGS=\"-D warnings\" cargo doc --workspace --no-deps"),
        "the canonical gate must reject rustdoc warnings"
    );

    let ci = fs::read_to_string(workflows.join("ci.yml")).expect("read CI workflow");
    for needle in [
        "nightly-2026-08-28",
        "cargo +nightly-2026-08-28 fuzz build",
        "cargo deny check --manifest-path fuzz/Cargo.toml",
        "cargo deny check --manifest-path spikes/grid-egui/Cargo.toml",
        "cargo deny check --manifest-path spikes/ironcalc/Cargo.toml",
    ] {
        assert!(ci.contains(needle), "CI workflow missing {needle}");
    }

    let nightly = fs::read_to_string(workflows.join("nightly.yml")).expect("read nightly");
    for needle in [
        "toolchain: nightly-2026-08-28",
        "cargo +nightly-2026-08-28 fuzz list",
        "cargo +nightly-2026-08-28 fuzz build",
        "test \"${#fuzz_targets[@]}\" -gt 0",
        "curl --fail --silent --show-error http://127.0.0.1:11434/api/tags",
    ] {
        assert!(nightly.contains(needle), "nightly workflow missing {needle}");
    }
    assert!(
        !nightly.contains("cargo +nightly "),
        "nightly commands must use the pinned dated toolchain"
    );
    let ready = nightly.find("/api/tags").expect("Ollama readiness probe");
    let pull = nightly.find("ollama pull").expect("Ollama model pull");
    assert!(ready < pull, "Ollama must be ready before pulling the model");

    let packaging =
        fs::read_to_string(workflows.join("packaging.yml")).expect("read packaging workflow");
    assert!(
        !packaging.contains("paths:"),
        "clean package validation must run for every pull request"
    );
    assert!(
        packaging.contains("scripts/arch-binary-package-smoke.sh"),
        "CI must build and install the binary PKGBUILD too"
    );

    let release = fs::read_to_string(workflows.join("release.yml")).expect("read release");
    let toolchain = release.find("Install Rust toolchain").expect("toolchain step");
    let metadata = release.find("cargo metadata").expect("metadata check");
    assert!(toolchain < metadata, "cargo metadata ran before toolchain setup");
    assert!(release.contains("mesa-vulkan-drivers"));
    assert!(release.contains("omacell-${version}.tar.gz"));
    assert!(
        !release.contains("/archive/refs/tags/"),
        "release recipes must not pin GitHub auto-generated archives"
    );
    assert!(release.contains("--notes-file release-notes.md"));
    assert!(!release.contains("--notes-file CHANGELOG.md"));
}

#[test]
fn wp28_packaging_and_rename_paths_are_reproducible() {
    let root = workspace_root();
    let source = fs::read_to_string(root.join("packaging/PKGBUILD")).expect("read PKGBUILD");
    let make_start = source.find("makedepends=(").expect("makedepends");
    let check_start = source.find("checkdepends=(").expect("checkdepends");
    assert!(
        source[make_start..check_start].contains("'python'"),
        "Python is used during build(), so it belongs in makedepends"
    );

    let binary =
        fs::read_to_string(root.join("packaging/PKGBUILD-bin")).expect("read binary PKGBUILD");
    assert!(!binary.contains("OMACELL_BIN_"));
    for needle in [
        "PKGBUILD_BIN_X86_64_URL",
        "PKGBUILD_BIN_AARCH64_URL",
        "PKGBUILD_BIN_X86_64_SHA256",
        "PKGBUILD_BIN_AARCH64_SHA256",
    ] {
        assert!(binary.contains(needle), "binary PKGBUILD missing {needle}");
    }

    let smoke =
        fs::read_to_string(root.join("scripts/arch-package-smoke.sh")).expect("read smoke");
    assert!(
        !smoke.contains("bsdtar -tf \"$package_file\" |"),
        "archive membership must not rely on a pipefail/SIGPIPE-sensitive pipeline"
    );

    let rename = fs::read_to_string(root.join("scripts/rename.sh")).expect("read rename");
    assert!(rename.contains("source packaging/name.env"));
    assert!(rename.contains("sha256sum \"packaging/${new_slug}.install\""));
    assert!(
        !rename.contains("s/Omacell/${new_display}/g"),
        "display-name replacement must not rewrite Rust identifiers"
    );

    let ignore = fs::read_to_string(root.join(".gitignore")).expect("read .gitignore");
    assert!(ignore.lines().any(|line| line == "**/*.snap.new"));
}

#[test]
fn wp28_review_named_parsers_have_fuzz_coverage() {
    let root = workspace_root();
    let manifest = fs::read_to_string(root.join("fuzz/Cargo.toml")).expect("read fuzz manifest");
    for target in ["application_parsers", "lua_runtime"] {
        assert!(
            manifest.contains(&format!("name = \"{target}\"")),
            "fuzz manifest missing {target}"
        );
        assert!(
            root.join(format!("fuzz/fuzz_targets/{target}.rs")).is_file(),
            "fuzz target source missing for {target}"
        );
    }
    let parsers = fs::read_to_string(root.join("fuzz/fuzz_targets/application_parsers.rs"))
        .expect("read application parser target");
    for needle in [
        "parse_plan",
        "parse_findings",
        "parse_plan_overlay",
        "parse_completion",
        "parse_and_eval",
        "parse_resource_uri",
        "parse_criteria",
        "parse_address",
        "parse_numeric_text",
        "parse_hypr_chords",
        "ShellTokens::parse",
        "serde_json::from_slice::<Chart>",
    ] {
        assert!(parsers.contains(needle), "fuzz target does not call {needle}");
    }
    let lua = fs::read_to_string(root.join("fuzz/fuzz_targets/lua_runtime.rs"))
        .expect("read Lua fuzz target");
    assert!(lua.contains("Runtime::new(Profile::Embedded"));
    assert!(lua.contains("runtime.exec("));
}
