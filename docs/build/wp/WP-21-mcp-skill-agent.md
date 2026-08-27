# WP-21 — MCP server, agent skill, and Omarchy agent hand-off

| | |
|---|---|
| Phase | 5 — Scripting, agents, AI |
| Lane | D — Integration (bus, CLI, Lua, MCP, AI, release) |
| Size | L (≈ 6–10) |
| Depends on | WP-07, WP-13, WP-19, WP-12 |
| Unblocks | WP-23, WP-28 |
| Spec sections | §7.6, §7.9, §8.5, §8.6 A-6.4 |
| Where | `crates/bus` (module `mcp`), `crates/cli` (`mcp`, `agent`), `default/agents/skills/omacell/SKILL.md` |

## Goal

Make Omacell an excellent tool for the user's coding agent — whichever one Omarchy's `default agent` names — without Omacell needing a model of its own.

## Deliverables

- MCP server (`rmcp`): stdio transport (`omacell mcp`) and socket (`--socket`); tools `workbook_open/list/save`, `sheet_list/add/rename`, `range_read` (values/formulas/formats, paginated), `range_write`, `formula_set`, `command_run`, `commands_list`, `recalc` (with `wait`), `audit`, `card` (stub returning summary level until WP-22), `changeset_propose/apply/revert/list`, `export`, `render` (returns 'GUI not running' when headless); resources `omacell://<file>/card` and `omacell://<file>/<sheet>`; write tools default to proposing changesets, `apply=true` only when policy allows.
- `SKILL.md` for the shipped skill: purpose, when to use, exact CLI cheatsheet (kept in sync with `omacell --help` by a test), MCP tool guidance, the propose-review-apply workflow, the done-checklist (`recalc --wait`, `audit --json`, `diff`), pitfalls (no direct file writes while a GUI has the file open; respect lock files).
- `omacell agent "<prompt>"`: detects Omarchy (`omarchy` on PATH and `omarchy default agent` set); runs `omarchy agent prompt` from the workbook's directory with workbook path, selection, and skill hints; off-Omarchy prints the equivalent command for the user's own harness; hidden from palette/status when no default agent.
- `omacell agent diagnose [--pid]`: builds the WP-19 diagnostic bundle (redacted via WP-22 when available) and hands it to the agent; status-line offers on `#REF!` cascade / circular reference / failed import gated by `[ai.agent] diagnose_offers` and by the presence of a default agent.
- Skill installation links created by `setup omarchy` (WP-12) into `~/.agents/skills/`, `~/.claude/skills/`, `~/.codex/skills/`, `~/.pi/agent/skills/`, `~/.gemini/config/skills/`; notifications through `omarchy-notification-send` when present (`changeset.proposed` from an external agent).

## Implementation notes

- Agents launched by Omarchy run unattended — the skill's default workflow must be *propose*, and `apply` must be a deliberate step.
- Test the hand-off with a fake `omarchy` script on `PATH` that records its arguments.

## Acceptance criteria

- [ ] MCP contract tests (an `rmcp` client) exercise every tool and resource against a fixture workbook, including pagination and error cases.
- [ ] Skill-drift test: every command mentioned in `SKILL.md` exists in the CLI help tree with the stated flags.
- [ ] Hand-off tests with the fake `omarchy`: arguments, cwd, hidden state without a default agent.
- [ ] A proposal from a headless MCP client appears in `omacell changeset list` and can be applied/reverted from the CLI.

## Tests

- MCP client tests; drift test; hand-off tests with fake PATH.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-21.md` before writing code.
4. Create branch `wp/21-mcp-skill-agent`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-21: MCP server, agent skill, and Omarchy agent hand-off`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
