# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Repository bootstrap (WP-00): Cargo workspace, CI, conventions, and crate skeletons.
- Merged work packages through WP-22 (engine, I/O, CLI/TUI/GUI, data tools, Lua, MCP/skill/agent, AI providers/privacy), plus WP-15a, WP-05F, WP-24–WP-26, WP-29, and spikes S1/S2. Per-package reports are in `reports/`.

### Changed

- Workspace `rust-version` is 1.98, matching `rust-toolchain.toml`, `.mise.toml`, and CI. The previous 1.85 declaration was unused and was blocking crate upgrades behind `deny.toml` ignores.
- `calamine` 0.36, `ratatui` 0.30 / `crossterm` 0.29, `fontdb` 0.24, `resvg`/`usvg` 0.48, `zbus` 5.19. This clears the HIGH `quick-xml` ignores (now 0.41 everywhere) and the unmaintained `paste` / `rustybuzz` ignores. The only remaining `deny.toml` advisory ignore is unmaintained `ttf-parser` (direct PDF embedding and egui's font stack), not an MSRV issue.
- IPC socket, task-runner, and Python bridge requests now share one mode, changeset, and origin policy dispatcher.
- The GUI wall-clock performance assertion runs nightly instead of gating pull requests on shared-runner timing.
- Criterion is 0.8 and GitHub Actions checkout is v7.

### Security

- Embedded Lua routes `print` through host status, uses a fixed command allowlist, cannot prompt or rebind keys, and documents its uncatchable instruction-budget errors.
- Agent hand-off content is stored in a private file instead of process argv, and option-like notification text bypasses positional helper arguments.
- Trust, config, theme, and keymap TOML parsers share a fuzz target; the trust store now uses the standard TOML parser.
