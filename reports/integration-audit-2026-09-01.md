# Integration audit — 1 September 2026

## Scope and evidence

This audit covers every completed report in `reports/` (`G1`, `WP-S1`,
`WP-S2`, `WP-00` through `WP-27`, and `WP-29`) on merged `main` at `f4159d2`
(conditional-format head `e00e7b6`). WP-28 and WP-30 have specifications but
no completion reports yet.

The audit:

- enumerated every unchecked checklist item;
- read every `Open questions / decisions needed` section;
- searched all report prose for deferred, missing, blocked, follow-up, and
  not-implemented statements;
- reconciled those statements with
  `docs/open-question-triage-2026-08-31.md`, the owning work-package spec, and
  current production/test sources;
- checked the live GitHub `main` ruleset (pull request plus strict required
  `check` status); and
- used the full green `just check` result from the IPC frame-limit integration
  stack and the prior green `cargo deny check` as the repository gate.

An unchecked box is not silently treated as complete. After this cleanup, the
only unchecked boxes in completed reports are the WP-15 terminal matrix and
WP-S2 CJK IME/Orca decision. Each is explicitly labeled **HUMAN / WP-28 G4**.
Merged frozen-contract approval markers in WP-22 and WP-24 were stale and are
now checked; the corresponding WP-17/WP-18 prose is also reconciled.

## Pre-WP-28 integration queue

These are product or cross-crate gaps, not packaging work. Complete them before
starting WP-28 unless a human changes their owner in the relevant report.

### P1 — engine and file fidelity

Completed in the first P1 follow-up: the formula printer now preserves lexical
`LET`/`LAMBDA` definition-site casing with shadowing, and all eight `*IF`
aggregates require references for their range positions while rejecting array
constants with `#VALUE!`.

Completed in the second P1 follow-up: live recalculation supplies omitted
`CELL()` references from the session's last changed cell; clipboard HTML decodes
all 2,125 semicolon-terminated HTML5 named references with bounded scanning; and
XLSX/OMC regression tests confirm both writers already rebuild output from live
workbook structures, excluding undo-only and evicted-history strings.

Completed in the third P1 follow-up: legacy multi-cell CSE formulas now retain a
fixed anchor range, recalculate without spilling, pad with `#N/A` or truncate to
that range, display `{=…}` from every member cell, survive undo/rebuild cleanup,
and round-trip through XLSX (`t="array"`) and OMC. Partial content and structural
edits that would split the fixed range are rejected before mutation. The frozen
v1 XLSX AI-formula bridge cannot encode CSE metadata, so that synthetic
combination is rejected in formula mode and flattened safely in values mode.

All P1 engine/file-fidelity items from this audit are complete.

### P2 — retained UI, IPC, and terminal integration

1. **Complete:** interactive saves now snapshot retained selection, scroll,
   zoom, freeze, split, and show-formulas state into the selected sheet
   `ViewState`, preserving workbook-owned gridlines and updating the live model
   only after the atomic file write succeeds. Lifecycle and pane-mode regression
   suites plus the full repository gate pass.
2. **Complete:** runner-backed IPC now implements the frozen subscribe and
   unsubscribe controls through independent count/byte-bounded filtered queues.
   Fan-out does not consume the retained GUI/TUI/Lua event queue, and overflow
   remains isolated to the stalled connection.
3. **Complete:** the frozen-contract RFC raises the hard/default IPC frame cap
   to 16 MiB and adds startup-only `[ipc].max_frame_bytes` tuning from 1–16 MiB.
   Server decode/encode, CLI clients, MCP, and the Python bridge enforce the
   same process limit; oversized records retain `ipc.frame` and direct callers
   to chunk large ranges. The connection cap remains 32 and IPC v1 envelopes
   are unchanged.
4. **Complete:** `[tui] graphics = auto` detects sixel/Kitty with a 75 ms
   bounded query and Kitty/Ghostty environment hints, then feeds the shared
   chart scene through a bounded background layout/raster/encoding worker. tmux/Herdr
   automatic mode and unsupported terminals use an ANSI Unicode-braille
   fallback; explicit passthrough remains available.
5. **Complete:** the required repository lint scans `docs/` for command-id
   shapes containing underscores while excluding documented filenames by known
   suffix. Frozen runtime parsing and documentation now enforce the same dotted
   lowercase command vocabulary.

### P3 — AI extension integration

1. Connect user-profile Lua `omacell.ai.task` plus request/response hooks to
   `AiRuntime`. Decide whether `omacell.ai.fn` gains an async-node adapter or
   remains an explicitly documented cache-only limitation.
2. Feed `ai.import.assist` output into the retained import-plan review UI; keep
   application explicit and reviewable.
3. Wire `[ai.functions] refresh_on_full_recalc` and AI-function `auto` through
   the retained hosts without weakening budget confirmation or autopilot scope.

`COPILOT()` remains inert on import. The design decision is HUMAN; the current
lean is an explicit one-key conversion, never automatic remapping.

## WP-28 entry gates and owned release work

The following items are intentionally not hidden in the integration queue:

- **Human/G4:** CJK IME, Orca speech, the expanded terminal matrix, integrated-
  GPU presentation-aware performance, and the final one-process-per-file ADR.
- **Human/G7:** Omarchy approval/trademark clearance for the name and the live
  Excel checklist/oracle rows that cannot be established locally.
- **WP-28:** packaging and Omarchy-channel CI; generated manual/reference drift;
  nightly fuzz; accessibility and i18n; semantic Quattro theme roles; all nine
  agent setup paths; the documented Hyprland launch-table form; GUI split-pane,
  font/fallback, shaping-cache and clickable-status closure; performance gates;
  print-title bands, printer palette, and PDF font policy.
- **WP-30:** repository security settings that require maintainer policy,
  including trusted-reviewer and emergency-bypass decisions.

## Explicit post-1.0 scope

These are deliberate product-scope decisions, not unexplained deferrals:

- Japanese-era/DBNum/Hijri number-format rendering unless a release corpus
  requires it;
- per-cell stale hatching during recalculation (busy status is the 1.0 policy);
- What-If Data Tables, Scenario Manager, and slicers as assigned by WP-18/24;
- `chartEx` exotic chart modeling and a full chart property inspector beyond
  the minimal move/resize/title release surface;
- ODS cross-sheet formula/number-format emission, Parquet date/time/decimal
  widening, and Parquet writing; and
- category consolidation and the other features explicitly marked Tier 1.

## Report-by-report disposition

| Report | Current disposition |
|---|---|
| `G1` | Engine gate complete; fixed-host/10% performance enforcement and remaining live Excel oracle rows are WP-28/HUMAN. |
| `WP-00` | Remote, ruleset, solo-review policy, and MIT are resolved; name/trademark and ADR-001 are explicit HUMAN gates. |
| `WP-01` | Open questions are resolved by the extended error table and formula-layer external-reference ownership. |
| `WP-02` | Structural rewrite, name grammar, file-boundary geometry, and live-only XLSX/OMC serialization are covered. |
| `WP-03` | Partial cut overlap, XFE names, 3-D qualifiers, and lexical lambda/LET definition casing are covered. |
| `WP-04` | Broadcast, What-If-table, and fixed-range legacy CSE decisions are covered; worst-case timing is a human perf-gate decision. |
| `WP-05F` | Zero-sized arrays and locale ownership are resolved. |
| `WP-05a` | CEILING/FLOOR, strict reference-only `*IF` ranges, and omitted-reference `CELL()` session state are covered. |
| `WP-05b` | Windows-1252, DATEDIF, YEARFRAC, and spill display decisions are resolved. |
| `WP-05c` | Duplicate metadata is closed; Excel-only oracle rows and fixed-host lookup baselines remain HUMAN/WP-28. |
| `WP-06` | Release behavior is resolved; rare calendar renderers are explicitly post-1.0. |
| `WP-07a` | Command ids, IPC origin policy, and number-format allocation are resolved. |
| `WP-07b` | Undo origin policy and the configurable 16 MiB frame-cap RFC are resolved. |
| `WP-08` | Open/recalc decisions and complete bounded HTML5 clipboard entities are covered. |
| `WP-09` / `WP-10` | Split units, later WP-17/18/25 ownership, and fixed-range CSE preservation are covered. |
| `WP-11` | OMC's readable/lossy-L3 policy and JSON-style writer decision are explicit. |
| `WP-12` / `WP-13` | Composition, signal, keymap, and IPC undo decisions are resolved. |
| `WP-14` | Deferred table is empty; count propagation, `ViewState` save synchronization, and the docs command-id lint are integrated. |
| `WP-15` | Runner/bootstrap and terminal-chart graphics are complete; terminal coverage is HUMAN/G4. |
| `WP-15a` | Worker/cancellation and runner-backed subscriptions are complete; stale-cell hatching is post-1.0. |
| `WP-16` | Clipboard and conditional formatting are integrated; remaining GUI completion and hardware decisions are WP-28/HUMAN. |
| `WP-17` / `WP-18` | Contracts were approved by merge; frontend clipboard/fill/drag and conditional-format consumers are integrated. |
| `WP-19` | WP-22 completed diagnostic redaction; search/error frontends are integrated. |
| `WP-20` | Retained Lua startup/events/keymaps/source integration is complete; AI-specific hooks are P3. |
| `WP-21` / `WP-22` | Agent/MCP and provider/privacy work is complete; WP-22 approval marker was reconciled. |
| `WP-23` | Acceptance work is complete; remaining Lua/import/refresh connections are P3 and `COPILOT()` is HUMAN. |
| `WP-24` / `WP-24a` | Pivot fidelity follow-up is complete; package-excluded Data Tables/Scenario Manager remain explicit post-1.0 scope. |
| `WP-25` | Chart export and terminal graphics are integrated; minimal release editing is WP-28 and exotic modeling is post-1.0. |
| `WP-26` | Print gaps are explicitly owned by WP-28. |
| `WP-27` | In-process XLS is complete; listed ODS/Parquet enhancements are explicit post-1.0 scope. |
| `WP-29` | Parser/security hardening is complete; repository-policy work is WP-30. |
| `WP-S1` | ADR-002 is decided. |
| `WP-S2` | Measurements are retained; CJK IME and Orca remain explicitly unchecked HUMAN/G4 gates. |
