# Changelog

All notable user-facing changes are recorded here. Omacell follows semantic
versioning after the 1.0 release; the current `0.0.0` version is unreleased.

## Unreleased

- Native spreadsheet engine with GUI, TUI, CLI, local IPC, MCP, and Lua clients.
- `.xlsx`/`.xlsm`, CSV/TSV, ODS, JSON, Parquet, HTML/Markdown, PDF, `.omc`, and
  read-only native `.xls` support without a LibreOffice runtime dependency.
- Formula, formatting, table, validation, conditional-format, pivot, chart,
  printing, comments, protection, data-tool, AI, and changeset workflows.
- Palette-visible chart move, resize, chart-title, and axis-title editing with
  undo/redo and reviewable changesets.
- Arch source/binary packaging, generated manual, Omarchy setup, parser
  hardening, accessibility coverage, localization scaffold, and release gates.
- Omarchy setup uses the Quattro launch-table form and links the shipped skill
  for all nine supported agent harnesses without replacing user-owned paths.

Release entries must state compatibility changes, migrations, security fixes,
and known fidelity differences. The tag workflow uses this file as its release
notes and refuses to publish until the external product-name gate is recorded.
