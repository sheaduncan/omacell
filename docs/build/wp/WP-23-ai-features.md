# WP-23 — AI features: cell functions, natural-language plans, formula assist, completion, import assist, AI audit, in-app agent

| | |
|---|---|
| Phase | 5 — Scripting, agents, AI |
| Lane | D — Integration (bus, CLI, Lua, MCP, AI, release) |
| Size | XL (≈ 10–20) |
| Depends on | WP-22, WP-04, WP-14, WP-10, WP-20, WP-21 |
| Unblocks | WP-28 |
| Spec sections | §8.3, §8.4, §8.6, §8.8, §13, §14 |
| Where | `crates/ai` (modules `functions`, `plan`, `formula`, `complete`, `import_assist`, `audit_ai`, `agent`), `default/ai/prompts/*.md`, `tests/evals/` |

## Goal

Put models where spreadsheet work happens, entirely through the command bus and changesets, with offline evals that prove behavior.

## Deliverables

- AI functions `AI`, `AI.EXTRACT`, `AI.CLASSIFY`, `AI.FILL`, `AI.TABLE`, `AI.TRANSLATE` as `AsyncNodeProvider` implementations: content-addressed cache (workbook custom part + `~/.cache/omacell/ai/`), batching (default 50 rows per request), budgets with confirmation events, provenance per result, `ai.refresh|pin|freeze` commands, `[ai.functions] xlsx_export` modes, `#N/A` with hints on failure, stale handling; `aicache` records in `.omc`.
- Natural-language plan task: prompt templates + structured output → validated command list → changeset proposal; the model sees only the registry schema and the card; palette `?` provider (WP-14 trait) and `Ctrl+Shift+A`/`:ai`.
- Formula tasks: `ai.formula.generate|explain|fix|refactor` with scratch-context evaluation before proposing; reference highlighting data for the UIs.
- Inline completion provider (`fast` slot; debounce; cancellation; `auto` mode only when local).
- Import assistant: proposes `ImportPlan` changes (WP-08) as a reviewable diff; never auto-applies.
- AI audit: takes WP-19 findings plus the card and adds judgments (unit mismatches from headers, suspicious constants) as findings with confidence; fixes are changesets.
- In-app agent loop: tool calling over the command bus with review mode; autopilot policy (scope, op caps, forbidden tools); conversation persisted in state dir (optionally in the workbook); retained panel model consumed by the WP-16/WP-15 frontends.
- Prompt templates under `default/ai/prompts/` (system + per task), versioned; user overrides in `~/.config/omacell/ai/prompts/`; Lua `omacell.ai.task` and `omacell.ai.fn` hooks; skills loading from `~/.config/omacell/ai/skills/` (ADR-006 format).
- Evals in `tests/evals/`: NL→plan tasks (≥ 200; scored on exact command match and on effect equivalence after execution), formula-generation tasks executed on fixture sheets, import-assist fixtures, audit precision/recall on seeded defects; the prompt-injection suite; all on recorded responses, with a nightly job against a local small model.

## Implementation notes

- No AI feature may write to the model except through a changeset; the property test from §14 is the gate.
- Cache determinism: same inputs, template version, and model → same result without a request.

## Acceptance criteria

- [x] All evals run offline in CI with pass rates recorded in the report; injection suite reports zero unexpected commands and zero policy changes.
- [x] Changeset invariants (review mode, apply/revert inverse, autopilot scope) hold under property tests.
- [x] A workbook with `AI.FILL` results saves to `.xlsx`, reopens in `calamine` and LibreOffice with the cached values, and re-opens in Omacell with formulas and provenance intact.
- [x] Budget confirmation triggers at the configured threshold; batching observed in recorded requests.

## Tests

- Eval runner with recorded fixtures; `proptest` invariants; round-trip tests; injection suite.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-23.md` before writing code.
4. Create branch `wp/23-ai-features`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-23: AI features: cell functions, natural-language plans, formula assist, completion, import assist, AI audit, in-app agent`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
