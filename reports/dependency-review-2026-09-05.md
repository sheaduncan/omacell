# Dependency PR review — 5 September 2026

## Plan (written before review fixes)

- Review every open Dependabot PR (#132–#139) against its exact upstream tag,
  migration notes, changed dependency graph, Omacell call sites, and current
  `main`. Keep every GitHub Action pinned to the full verified commit SHA.
- Land the workflow-only updates first (#132–#134), then the smaller Rust
  updates (#135–#137), updating each branch to the preceding merge so workflow
  and lockfile conflicts are resolved explicitly rather than by an unreviewed
  merge result.
- Treat major versions as migrations, not lockfile refreshes. PR #138 must
  preserve the embedded-Lua trust, memory, instruction, prompt, keymap, output,
  and command-capability boundaries. PR #139 must use one compatible
  `eframe`/`egui`/`egui_kittest` family and preserve input, accessibility,
  rendering, clipboard, and snapshot behavior.
- Add or adjust regressions before compatibility implementation when a changed
  API exposes behavior that the existing suite does not cover. Do not loosen,
  delete, or ignore an existing test to make an upgrade pass.
- For each PR, run the smallest relevant tests while iterating, then the exact
  `just check` gate. Require green hosted CI, clean Arch binary/source package
  jobs, and CodeQL before merge. Run `cargo deny check` for changed Rust
  dependency graphs.
- Update this report after each merge with the reviewed behavior, test
  evidence, dependency-graph impact, and any remaining queue item.

## Initial triage

| PR | Upgrade | Initial review status |
|---:|---|---|
| #132 | `actions/upload-artifact` 4.6.2 → 7.0.1 | Previously green on the old base. The three artifact names are unique, paths contain only intended release/performance evidence, and the existing inputs remain supported. Recheck on current `main`. |
| #133 | `actions/download-artifact` 4.3.0 → 8.0.1 | Previously green on the old base. `pattern` plus `merge-multiple` remains supported; v8 now rejects digest mismatches by default, which is the desired release posture. Recheck the end-to-end release artifact flow on current `main`. |
| #134 | `taiki-e/install-action` 2.87.0 → 2.87.4 | Previously green on the old base. Patch release changes unrelated tool recipes; verify the pinned SHA and all repository-requested tools. |
| #135 | `indexmap` 2.14.0 → 2.14.2 | Lockfile-only patch selected by the existing workspace range. Verify deterministic-order tests and the resolved graph. |
| #136 | `clap_mangen` 0.2.33 → 0.3.3 | Dev/build-time major update. Verify generated man pages and CLI reference drift, not only compilation. |
| #137 | `toml_edit` 0.22.27 → 0.25.13 | Direct parser/editor major update. Verify config migration, formatting/comment preservation, symlink containment, and parser limits. |
| #138 | `mlua` 0.10.5 → 0.12.1 | Existing CI fails. Perform an API migration and re-run the full embedded/interactive Lua sandbox suite. |
| #139 | `eframe` 0.32.3 → 0.36.1 | Existing CI fails. Dependabot upgraded `eframe` alone, producing parallel egui/wgpu/accessibility graphs while direct `egui` and `egui_kittest` remain 0.32; replace this with a coordinated GUI-stack migration. |

Primary upstream references used during review:

- [`actions/upload-artifact` v7.0.1](https://github.com/actions/upload-artifact/releases/tag/v7.0.1)
- [`actions/download-artifact` v8.0.1](https://github.com/actions/download-artifact/blob/v8.0.1/README.md)
- [`taiki-e/install-action` v2.87.4](https://github.com/taiki-e/install-action/releases/tag/v2.87.4)
- [`indexmap` 2.14.2](https://github.com/indexmap-rs/indexmap/releases/tag/2.14.2)
- [`mlua` v0.12.1](https://github.com/mlua-rs/mlua/releases/tag/v0.12.1)
- [`egui` 0.36.1](https://github.com/emilk/egui/releases/tag/0.36.1)

## What was reviewed and changed

- **#132 — `actions/upload-artifact` 7.0.1:** verified that the pinned
  `043fb46d1a93c77aae656e7c1c64a875d1fc6a0a` commit is the signed v7.0.1
  release, that all configured inputs remain part of the action contract, and
  that the source, architecture, and fixed-host artifact names are unique.
  The action now declares the Node 24 runtime (Actions Runner 2.327.1 or
  newer). Release uploads run on GitHub-hosted images; the fixed-host lane
  already starts with the Node-24-based `actions/checkout` v7, so this update
  does not introduce a new runner-runtime requirement for that machine.
- **#133 — `actions/download-artifact` 8.0.1:** verified that the pinned
  `3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c` commit is the signed v8.0.1
  release and that `pattern` and `merge-multiple` retain their prior meaning.
  V8 changes digest mismatches from a warning to an error by default. The
  release workflow now requests that behavior explicitly, and every release
  upload explicitly rejects a missing artifact path. A repository-lint
  regression locks those fail-closed handoff settings in place.
- **#134 — `taiki-e/install-action` 2.87.4:** verified the immutable v2.87.4
  release and its `e67fa11c4b9316fa714ddf0abed07a0c3143b95b` commit. The
  release changes only the `uv`, `protoc`, and `coreutils` recipes, while
  Omacell requests `just`, `cargo-audit`, `cargo-deny`, and `cargo-fuzz`.
  All three workflow uses remain full-SHA pinned and keep their existing tool
  lists.
- **#135 — `indexmap` 2.14.2:** Dependabot's stale title says 2.14.1, but the
  reviewed lockfile correctly resolves the newer signed 2.14.2 patch under the
  workspace's `version = "2"` requirement. Upstream 2.14.1 contains internal
  comparison/assertion cleanup; 2.14.2 fixes caller-name hygiene in map/set
  macros and permits const empty macro initialization. Omacell uses ordinary
  `IndexMap` values, not the changed macros, and the update adds no package or
  second indexmap version.
- **#136 — `clap_mangen` 0.3.3:** reviewed the complete 0.3 changelog. The
  only compatibility switch requires an opt-in `env` feature for clap
  environment annotations; Omacell declares none. The later releases improve
  synopsis handling for required groups, override usage, and hidden
  positionals without changing the `Man::new(...).render(...)` API used here.
  Regenerating the complete `omacell.1` page produced the exact same SHA-256
  (`ff4aa08fc8e7c51cbbc877169d6e703a88be660a80aa1f070e6e2ac2b4cc5a77`),
  so this CLI currently exercises none of those output changes.
- **#137 — `toml_edit` 0.25.13:** reviewed the 0.23–0.25 parser/writer
  migration, including the new TOML 1.1 grammar and the upstream recursion,
  overflow, and malformed-inline-table fixes. The public API used by Omacell
  still compiles, and the existing migration tests preserve comments, layout,
  permissions, backups, and symlink identity. Review found one behavior gap:
  the AI setup editor would accept and modify TOML 1.1 input that the
  application's TOML 1.0 loader rejects. The editor now validates with the
  loader grammar before the format-preserving parse and leaves rejected files
  byte-identical. The fuzz-workspace lock now records the new `toml_writer`
  graph instead of rewriting itself in CI.
- **#138 — `mlua` 0.12.1:** reviewed the complete 0.11–0.12 migration notes,
  including the Lua 5.4.8 update, hook changes, userdata/thread changes, and
  newly fallible metatable operations. Omacell's public Lua API and sandbox
  profiles remain unchanged. The runtime now propagates failures while
  installing the embedded instruction hook and JSON-array metatable instead
  of silently discarding them. The independently committed fuzz lock is also
  synchronized to `mlua` 0.12.1 and its updated vendored Lua/tool graph.
- **#139 — `eframe`/`egui`/`egui_kittest` 0.36.1:** replaced Dependabot's
  partial `eframe` update with one coordinated GUI stack, eliminating the
  parallel 0.32 and 0.36 egui/wgpu/accessibility graphs. Migrated the root-UI
  application lifecycle, unified panel and global-style APIs, semantic scroll
  and zoom input, asynchronous wgpu adapter discovery, and the snapshot result
  collector. A new lifecycle regression locks Ctrl-wheel zoom to the existing
  `view.zoom` command path. Upstream's new font shaping/rendering stack changes
  raster output, so all nine theme/scale baselines were regenerated only after
  the accessibility and crisp-grid assertions passed and representative light,
  dark, 1x, 1.5x, and 2x renders were inspected.

## Interfaces exposed

None. Dependency migrations must preserve Omacell's frozen public and wire
contracts.

## Deviations with reasons

None yet.

## Measurements

- #132: clean current-`main` merge; `git diff --check`; exact `just check`
  passed. Hosted `check`, CodeQL, action analysis, Rust/Python analysis, and
  clean Arch binary/source package jobs all passed; the PR was merged.
- #133: `release_artifact_handoff_is_explicitly_fail_closed` failed before the
  workflow hardening (zero of two uploads rejected missing files), then passed
  after the implementation. The exact local `just check` gate and hosted
  `check`, CodeQL, all analysis, and both clean Arch package jobs passed; the
  PR was merged.
- #134: signed-tag/SHA verification, `git diff --check`, and the exact local
  `just check` gate passed. Hosted gates are pending.
- #135: `cargo deny check` passed (only the already-allowed duplicate/license
  warnings); all tests and doctests for the direct consumer crates
  `omacell-core`, `omacell-bus`, `omacell-io`, and `omacell-ui` passed. The
  first sandboxed run could not bind Unix sockets; the identical permitted run
  passed, including all 24 IPC server tests. The exact local `just check` gate
  passed; hosted gates are pending.
- #136: the focused distribution test passed, generated all Bash, Fish, Zsh,
  and man release files, and produced a byte-identical man page; its digest is
  recorded above.
  The separately committed fuzz-workspace lockfile was synchronized to 0.3.3.
  `cargo deny check` passed for both the root and fuzz graphs with only the
  repository's already-allowed duplicate/license warnings. The exact local
  `just check` gate passed; hosted gates are pending.
- #137: the TOML 1.1 mismatch regression failed before the grammar guard and
  passed afterward. All `omacell-conf` unit, integration, migration, theme,
  setup, watcher, and doc tests passed; `cargo deny check` passed for the root
  graph. The combined config/theme/keymap/trust parser fuzz target completed
  10,000 cases without a crash, and fuzz-graph `cargo deny` passed. The exact
  local `just check` gate passed; hosted gates are pending.
- #138: all 39 `omacell-lua` API, interactive-runtime, recorder, sandbox, and
  trust tests passed. Root and fuzz-graph `cargo deny` checks passed with only
  the repository's already-allowed duplicate/license warnings. The Lua runtime
  fuzz target completed 10,000 cases without a crash; LeakSanitizer was
  disabled for that local run because this container's ptrace policy prevents
  LeakSanitizer startup, while the target's AddressSanitizer instrumentation
  remained enabled. The exact local `just check` gate passed; hosted gates are
  pending.
- #139: all 15 GUI unit tests, five enabled accessibility tests, 28 lifecycle
  tests, two reload tests, the nine-case snapshot test, and doc tests passed;
  the two fixed-host-only gates remained ignored as designed. The new
  Ctrl-wheel regression passed, all nine snapshot baselines passed the updated
  comparator, and root/fuzz `cargo deny check` passed with only the
  repository's allowed duplicate/license warnings. The exact local
  `just check` gate passed; hosted gates are pending.

## Open questions

None. A major upgrade that cannot preserve the documented behavior and gates
will be closed with a recorded reason rather than merged partially.

## RFC

None. No frozen contract change is authorized by these dependency updates.

## Checklist

- [x] PR #132 reviewed and merged
- [x] PR #133 reviewed and merged
- [ ] PR #134 reviewed and merged
- [ ] PR #135 reviewed and merged
- [ ] PR #136 reviewed and merged
- [ ] PR #137 reviewed and merged
- [ ] PR #138 migrated, reviewed, and merged
- [ ] PR #139 migrated as one compatible GUI stack, reviewed, and merged
- [ ] `cargo deny check` green for each changed Rust graph
- [ ] Exact `just check`, hosted CI, clean packages, and CodeQL green
- [ ] No open PR remains from the initial #132–#139 queue
