# Integration audit — 1 September 2026

## Scope and evidence

This audit covers every completed report in `reports/` (`G1`, `WP-S1`,
`WP-S2`, `WP-00` through `WP-29`) through the final pre-release reconciliation
and the WP-28 release branch. WP-30 has a specification but no completion
report and remains separately owned repository-policy work.

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
- reran the repository/report regression lints, focused panel/frontend tests,
  the full `just check`, and `cargo deny check` after the final reconciliation.

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

1. **Complete:** user-profile Lua tasks and request/response hooks are installed
   atomically in `AiRuntime`; `omacell.ai.fn` opts into the async graph through
   the source-compatible `DynamicFnBody::async_node()` default method. GUI/TUI
   settle new pending generations off-thread and queue an incremental settlement
   wave on the single writer. Embedded scripts retain no AI extension capability.
2. **Complete:** CSV/TSV opens retain the sniffed `ImportPlan` and bounded
   preview in GUI/TUI. `A` explicitly sends a provider-policy-filtered preview
   to `ai.import.assist`; the proposal remains unapplied until Enter atomically
   reopens the source with the reviewed plan. Schema policy strips sample
   values, detector redaction precedes hooks/providers, and stale results fail
   closed.
3. **Complete:** `[ai.functions] auto` gates cache-miss requests to changed
   inputs, while `ai.refresh` and configured user-requested full recalculation
   explicitly authorize re-query. File-open, startup Lua sourcing, and
   `on_open` establish AI-cell hashes without sending; retained config reloads
   update function limits and policy. Pending nodes survive for the incremental
   settlement wave, so budget confirmation and non-volatile cache semantics are
   preserved without a full-recalc loop.

`COPILOT()` remains inert on import. The design decision is HUMAN; the current
lean is an explicit one-key conversion, never automatic remapping.

### Final report and presentation reconciliation

1. **Complete:** GUI/TUI no longer carry WP-era format/comments/sort/filter
   placeholders. `format.panel` consumes the live bus result; comments and
   filter panels read the immutable workbook snapshot; sort shows the active
   selection and typed command payload. The three new closed-empty panel
   commands are palette-visible, session-only, and share one toolkit-neutral
   implementation.
2. **Complete:** GUI row/column drag and double-click auto-fit update the
   viewport immediately and then submit `format.rowheight` / `format.colwidth`
   to the single writer, so undo, dirty state, and file save retain the change.
   A shared frontend policy ensures workbook-mutating `view.*`/`edit.*`
   commands are no longer mistaken for session-only commands; both frontends
   refresh committed geometry after resize, auto-fit, hide/unhide, structural
   edits, and undo.
3. **Complete:** unused CLI stub exit scaffolding was removed; low-level row
   shift docs now point to the formula-aware `core::ops` path; production
   redaction no longer uses `unwrap`/`expect` to compile built-in patterns.
4. **Complete:** stale dependency-advisory claims, already-merged RFC gates,
   WP-24a fidelity gaps, and historical ownership statements were reconciled
   against current source and lockfiles. Repository lint now permits exactly
   the four explicitly owned HUMAN / WP-28 G4 boxes and rejects pending merge
   gates in completed reports/contracts.

The pre-WP-28 product and cross-crate integration queue is empty after PR69 and
this reconciliation merge. Work remaining below is intentionally owned by
WP-28, WP-30, post-1.0 scope, or a named human gate.

The three new panel command ids are additive changes to the frozen WP-07a
catalog registered by the retained GUI/TUI composition roots. Their schemas are
closed and empty, headless composition roots remain unchanged, existing
ids/envelopes are unchanged, and merge of this reconciliation is the human
approval record.

## WP-28 technical closure and owned live gates

The following items are intentionally not hidden in the integration queue:

- **Human/G4:** CJK IME, Orca speech, the expanded terminal matrix, integrated-
  GPU presentation-aware performance, and the final one-process-per-file ADR.
- **Human/G7:** Omarchy approval/trademark clearance for the name and the live
  Excel checklist/oracle rows that cannot be established locally.
- **Human/G5:** decide whether agent hand-offs retain the workbook directory as
  cwd for file context or use Omarchy's `~/Work` trust-persistence convention.
- **Completed in WP-28:** packaging and Omarchy-channel workflow definitions;
  generated manual/reference drift; nightly fuzz; accessibility and i18n;
  semantic Quattro theme roles; all nine agent-specific/generic skill links;
  the documented Hyprland launch-table form; GUI split-pane, font/fallback,
  shaping-cache and clickable-status closure; performance gates; print-title
  bands, printer palette and PDF font policy; and the minimal chart
  move/resize/title/axis-title command surface.
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
| `WP-11` | OMC's readable/lossy-L3 policy and JSON-style writer decision are explicit; typed later-package records and CLI export are reconciled. |
| `WP-12` / `WP-13` | Composition, signal, keymap, and IPC undo decisions are resolved. |
| `WP-14` | Deferred table is empty; count propagation, `ViewState` save synchronization, and the docs command-id lint are integrated. |
| `WP-15` | Runner/bootstrap, terminal-chart graphics, and live comments/format/sort/filter panels are complete; terminal coverage is HUMAN/G4. |
| `WP-15a` | Worker/cancellation and runner-backed subscriptions are complete; stale-cell hatching is post-1.0. |
| `WP-16` | Clipboard, conditional formatting, split panes, font-family caching, and clickable status are integrated; remaining live hardware decisions are HUMAN/G4. |
| `WP-17` / `WP-18` | Contracts were approved by merge; frontend clipboard/fill/drag, panel consumers, and conditional-format consumers are integrated. |
| `WP-19` | WP-22 completed diagnostic redaction; search/error frontends are integrated. |
| `WP-20` | Retained Lua startup/events/keymaps/source integration and user-profile AI extensions are complete; merged async-node approval is reconciled. |
| `WP-21` / `WP-22` | Agent/MCP and provider/privacy work is complete; hand-off content uses a private file and merged approval markers are reconciled. |
| `WP-23` | Acceptance, Lua AI, import review, and AI-function refresh/lifecycle integration are complete; `COPILOT()` remains an explicit HUMAN decision. |
| `WP-24` / `WP-24a` | Pivot fidelity follow-up is complete; package-excluded Data Tables/Scenario Manager remain explicit post-1.0 scope. |
| `WP-25` | Chart export, terminal graphics, and minimal release editing are integrated; exotic modeling and a full property inspector are post-1.0. |
| `WP-26` | Explicit print-title bands, accessible printer selection, embedded system fonts, and the documented Helvetica fallback are complete. |
| `WP-27` | Native XLS is complete through the bundled private, resource-limited worker; listed ODS/Parquet enhancements are explicit post-1.0 scope. |
| `WP-28` | Agent-verifiable packaging, docs, workflow, hardening, a11y, i18n, GUI, print, chart, and release work is complete; named live runners, hardware walkthroughs, and public-name clearance remain HUMAN gates. |
| `WP-29` | Parser/security hardening is complete; repository-policy work is WP-30. |
| `WP-S1` | ADR-002 is decided. |
| `WP-S2` | Measurements are retained; CJK IME and Orca remain explicitly unchecked HUMAN/G4 gates. |
