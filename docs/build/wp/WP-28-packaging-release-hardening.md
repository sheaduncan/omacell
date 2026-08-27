# WP-28 — Packaging, documentation, Omarchy CI, hardening, accessibility, i18n scaffolding, release

| | |
|---|---|
| Phase | 7 — Release |
| Lane | D — Integration (bus, CLI, Lua, MCP, AI, release) |
| Size | L (≈ 6–10) |
| Depends on | WP-13, WP-15, WP-16, WP-21, WP-23, WP-24, WP-25, WP-26, WP-27 |
| Unblocks | — |
| Spec sections | §7.6, §7.8, §12, §14, §15 (1.0), §16 |
| Where | `packaging/`, `docs/`, `.github/`, `crates/*` (hardening) |

## Goal

Ship: an Arch package that installs cleanly on Omarchy, documentation that matches the binary, CI that tracks Omarchy's channels, and the hardening passes the spec promises.

## Deliverables

- `PKGBUILD` (source) and `-bin` variant; `.desktop` with MIME associations (`.xlsx`, `.xlsm`, `.csv`, `.tsv`, `.ods`, `.omc`), MIME XML, `hicolor` icons plus symbolic variant, completions/man/skill/default files installed under `/usr/share/omacell`; post-install message pointing to `omacell setup omarchy`; `makepkg` build in an Arch container in CI.
- Documentation (`mdBook`): user manual, configuration reference generated from `config.schema.json`, CLI reference generated from `clap`, Lua API from WP-20, AI and privacy chapter, Omarchy integration chapter with the Hyprland/menu snippets; a drift test between generated references and the binary.
- Omarchy CI job: VM images for `stable`, `RC`, `edge`; installs the package; runs `omacell setup omarchy`; switches themes/fonts/text size; picks a default agent (fake); asserts the §7 and §8.5 behaviors via CLI/IPC; theme-fixture refresh script.
- Performance gate workflow: the §12.1 table as benches with committed baselines and a 10 % budget; `just perf-baseline` documented.
- Hardening: fuzz targets wired for formula, numfmt, xlsx zip/xml, csv, omc, IPC decoder; `cargo audit`/`deny` in CI; parser limits documented.
- Accessibility pass: AccessKit tree checks for grid, panels, palette; keyboard-only walkthrough test; reduced-motion respected. i18n scaffolding: Fluent bundles for UI strings (`en-US`), extraction script, no hard-coded user-facing strings (lint).
- Rename readiness: the product name in one constant and one packaging variable, with a `scripts/rename.sh`; release workflow (tag → build → artifacts → changelog).

## Implementation notes

- Humans decide the final name and license before the first public tag; this package must not block on them.
- Do not write into `/usr/share/omarchy` or `~/.config/omarchy` from the package; only `setup omarchy` (user-run) touches the latter.

## Acceptance criteria

- [ ] `makepkg` succeeds in a clean Arch container; installed binary passes the CLI smoke suite; `.desktop` validates (`desktop-file-validate`).
- [ ] Omarchy VM job green on all three channels; docs build with zero drift failures; perf gates green with committed baselines.
- [ ] Fuzz targets run nightly; a11y tree tests pass; string-extraction lint passes.

## Tests

- Container build test; VM integration job; drift tests; perf gates; nightly fuzz.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-28.md` before writing code.
4. Create branch `wp/28-packaging-release-hardening`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-28: Packaging, documentation, Omarchy CI, hardening, accessibility, i18n scaffolding, release`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
