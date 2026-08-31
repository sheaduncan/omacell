# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Repository bootstrap (WP-00): Cargo workspace, CI, conventions, and crate skeletons.
- Merged work packages through WP-21 (engine, I/O, CLI/TUI/GUI, data tools, Lua, MCP/skill/agent), plus WP-15a, WP-05F, WP-24–WP-26, WP-29, and spikes S1/S2. Per-package reports are in `reports/`.

### Changed

- Workspace `rust-version` is 1.98, matching `rust-toolchain.toml`, `.mise.toml`, and CI. The previous 1.85 declaration was unused and was blocking crate upgrades behind `deny.toml` ignores.
- `calamine` 0.36, `ratatui` 0.30, `fontdb` 0.24, `resvg`/`usvg` 0.48, `zbus` 5.19. This clears the HIGH `quick-xml` ignores (now 0.41 everywhere) and the unmaintained `paste` / `rustybuzz` ignores. The only remaining `deny.toml` advisory ignore is unmaintained `ttf-parser` (PDF embed / `fontdb`), not an MSRV issue.
