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
- used the full green `just check` and `cargo deny check` results from the
  conditional-format integration stack as the repository gate.

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

1. Preserve multi-cell legacy CSE formulas end to end. The current `ARRAY` flag
   prevents spilling but carries no fixed range; XLSX import/write therefore
   cannot preserve `<f t="array" ref="…">` semantics.
2. Add `Workbook::compact_interners()` (or an equivalent bounded writer pass)
   before XLSX/OMC serialization so evicted undo history cannot leave dead
   shared strings in saved files.
3. Connect omitted-reference `CELL()` to the last changed cell retained by the
   live session. The current corpus intentionally uses the formula cell as a
   known difference.
4. Decode the full bounded HTML5 named-entity table for clipboard HTML. The
   current test intentionally leaves `&eacute;` undecoded.

### P2 — retained UI, IPC, and terminal integration

1. Flush the retained selection, scroll position, zoom, freeze, and split state
   into the active sheet `ViewState` before save. Both frontends hydrate those
   fields, but no reverse synchronization exists.
2. Add bounded subscribe/unsubscribe support to runner-backed IPC. The frozen
   control operations and bus-backed server already support it; live GUI/TUI
   use `serve_runner` and currently cannot expose that event stream.
3. Resolve the triaged IPC frame-limit decision through a frozen-contract RFC:
   raise the 1 MiB cap to a configurable 16 MiB as recommended, or document and
   test a chunking contract that covers 100k-cell operations.
4. Complete `[tui] graphics = auto` detection and the sixel/kitty chart path.
   The current function only returns explicit `sixel`/`kitty`; `auto` is a
   named hook with no rendered chart consumer.

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
| `WP-02` | Structural rewrite, name grammar, and file-boundary geometry decisions are resolved; dead-interner compaction is P1. |
| `WP-03` | Partial cut overlap, XFE names, 3-D qualifiers, and lexical lambda/LET definition casing are covered. |
| `WP-04` | Broadcast and What-If-table decisions are covered; multi-cell CSE is P1 and worst-case timing is a human perf-gate decision. |
| `WP-05F` | Zero-sized arrays and locale ownership are resolved. |
| `WP-05a` | CEILING/FLOOR and strict reference-only `*IF` range handling are fixed; omitted `CELL()` session state remains P1. |
| `WP-05b` | Windows-1252, DATEDIF, YEARFRAC, and spill display decisions are resolved. |
| `WP-05c` | Duplicate metadata is closed; Excel-only oracle rows and fixed-host lookup baselines remain HUMAN/WP-28. |
| `WP-06` | Release behavior is resolved; rare calendar renderers are explicitly post-1.0. |
| `WP-07a` | Command ids, IPC origin policy, and number-format allocation are resolved. |
| `WP-07b` | Undo origin policy is resolved; frame-cap/chunking is P2 and requires an RFC. |
| `WP-08` | Open/recalc decisions are resolved; complete HTML entities are P1. |
| `WP-09` / `WP-10` | Split units and later WP-17/18/25 ownership are resolved; fixed-range CSE preservation remains P1. |
| `WP-11` | OMC's readable/lossy-L3 policy and JSON-style writer decision are explicit. |
| `WP-12` / `WP-13` | Composition, signal, keymap, and IPC undo decisions are resolved. |
| `WP-14` | Deferred table is empty and count propagation is integrated; `ViewState` flush and docs command-id lint are P2. |
| `WP-15` | Runner/bootstrap follow-ups are complete; graphics is P2 and terminal coverage is HUMAN/G4. |
| `WP-15a` | Worker/cancellation is complete; runner-backed subscriptions are P2; stale-cell hatching is post-1.0. |
| `WP-16` | Clipboard and conditional formatting are integrated; remaining GUI completion and hardware decisions are WP-28/HUMAN. |
| `WP-17` / `WP-18` | Contracts were approved by merge; frontend clipboard/fill/drag and conditional-format consumers are integrated. |
| `WP-19` | WP-22 completed diagnostic redaction; search/error frontends are integrated. |
| `WP-20` | Retained Lua startup/events/keymaps/source integration is complete; AI-specific hooks are P3. |
| `WP-21` / `WP-22` | Agent/MCP and provider/privacy work is complete; WP-22 approval marker was reconciled. |
| `WP-23` | Acceptance work is complete; remaining Lua/import/refresh connections are P3 and `COPILOT()` is HUMAN. |
| `WP-24` / `WP-24a` | Pivot fidelity follow-up is complete; package-excluded Data Tables/Scenario Manager remain explicit post-1.0 scope. |
| `WP-25` | Chart export is integrated; terminal graphics is P2, minimal release editing is WP-28, exotic modeling is post-1.0. |
| `WP-26` | Print gaps are explicitly owned by WP-28. |
| `WP-27` | In-process XLS is complete; listed ODS/Parquet enhancements are explicit post-1.0 scope. |
| `WP-29` | Parser/security hardening is complete; repository-policy work is WP-30. |
| `WP-S1` | ADR-002 is decided. |
| `WP-S2` | Measurements are retained; CJK IME and Orca remain explicitly unchecked HUMAN/G4 gates. |
