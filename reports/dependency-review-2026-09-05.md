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

Pending.

## Interfaces exposed

None. Dependency migrations must preserve Omacell's frozen public and wire
contracts.

## Deviations with reasons

None yet.

## Measurements

Pending per-PR gate evidence.

## Open questions

None. A major upgrade that cannot preserve the documented behavior and gates
will be closed with a recorded reason rather than merged partially.

## RFC

None. No frozen contract change is authorized by these dependency updates.

## Checklist

- [ ] PRs #132–#134 reviewed and merged
- [ ] PRs #135–#137 reviewed and merged
- [ ] PR #138 migrated, reviewed, and merged
- [ ] PR #139 migrated as one compatible GUI stack, reviewed, and merged
- [ ] `cargo deny check` green for each changed Rust graph
- [ ] Exact `just check`, hosted CI, clean packages, and CodeQL green
- [ ] No open PR remains from the initial #132–#139 queue
