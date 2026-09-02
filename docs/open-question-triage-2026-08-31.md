# Open-question triage — 31 August 2026

> This is the decision snapshot used for the follow-up work. Current completion,
> explicit post-1.0 scope, human gates, and the remaining pre-WP-28 queue are
> tracked in `reports/integration-audit-2026-09-01.md`.

Resolves the 83 `## Open questions` bullets across `reports/WP-*.md` against
Excel 365 semantics and Omarchy 4.0 "Quattro" (released 14 Aug 2026).
Each item carries a **Decision**, the evidence, a confidence mark, and an
owner tag:

- `CLOSED` — already resolved by a later package; delete from the report or mark answered.
- `AGENT` — decided here; an agent can encode it (corpus row, fix, doc) in WP-28 or a small follow-up WP.
- `HUMAN` — needs you: a live Excel 365 check, real hardware, or a product call.
- `BUG` — the current implementation is wrong per the evidence.

Confidence: **H** = documented Excel behaviour or ECMA-376 text; **M** = well-known
behaviour worth one live check; **L** = judgment call. Everything marked "verify
live" is collected into Appendix A as a 10-minute Excel checklist.

Counts: 21 CLOSED · 41 AGENT · 16 HUMAN · 5 BUG.

---

## Quattro facts that bear on these answers

Verified against the v4.0.0 release notes and the Omarchy manual:

- **First-party "Oma-" app family.** Quattro ships Omawrite, Omacut, and Omacalc as default apps, C++/Qt, hosted under the `omacom` GitHub org and distributed through the Omarchy Package Repository. Omacalc is a four-function calculator, not a spreadsheet — no functional collision — but "Oma-" now reads as *first-party*. See WP-00.3.
- **Default terminal is Foot** (sixel capable). Herdr ships alongside tmux. Ghostty/Kitty speak the kitty graphics protocol. See WP-15.1, WP-15.3.
- **Hyprland config is Lua.** The idiomatic app binding is `o.bind("SUPER + SHIFT + W", "Omawrite", { launch = "omawrite" })`. WP-28 updated Omacell's emitted snippet to the equivalent `{ launch = "omacell" }` table form.
- **Apps run in their own systemd scopes** with `systemd-oomd` allowed to kill a runaway app. Favors process-per-file (WP-16.1) and makes autosave/recovery load-bearing.
- **Theme:** state at `~/.local/state/omarchy/current/theme/colors.toml`, expanded from 8 to 24 semantic colors. WP-28 maps the available semantic surface/text/state/reference/chart keys into GUI roles, retaining derived fallbacks only when a theme omits a role.
- **Text scaling:** `omarchy display text size` moves GTK's `text-scaling-factor`, which `conf/font.rs` already reads. Test 9px and 20px extremes at G4.
- **Notifications** are native to the Quickshell shell; the freedesktop D-Bus fallback in `conf/notify.rs` is the right path, `omarchy-notification-send` if still present is a bonus.
- **Default agent:** nine harnesses selectable (Claude Code, Codex, OpenCode, Pi, Oh My Pi, Gemini, Grok, Copilot, Crush), launched as `org.omarchy.agent`, started in `~/Work` "so trust sticks". WP-28 links the skill into the generic and harness-specific Claude, Codex, OpenCode, Pi, Gemini, Grok, Copilot, and Crush locations. Omacell's hand-off uses the workbook directory as cwd, which means a per-directory trust prompt in Claude Code — decide at G5 whether to follow Omarchy's `~/Work` convention instead.
- **Menu extension** path `~/.config/omarchy/extensions/omarchy-menu.jsonc` — already what `setup.rs` writes.
- **Security posture:** Quattro closed three code-execution paths that a malicious *theme* could take. Treat `colors.toml` as untrusted input (it already goes through the `toml` crate — good).

---

## Phase 0

### WP-00.1 Create the GitHub remote and branch protection
`CLOSED` — remote exists and is public. Verify protection requires the `check` status. Note "never merge your own PR" cannot hold on a solo repo; every merge to date is self-merged. Either strike the rule or add a second reviewing identity.

### WP-00.2 Confirm MIT (D10)
`HUMAN` · **Decision: MIT, confirm now.** Omarchy and its first-party apps are MIT/free software; anything copyleft would be the odd one out in the ecosystem and complicates the Omarchy Package Repository path. Remove "Placeholder" from README and D10.

### WP-00.3 "Oma-" prefix / trademark
`HUMAN` · **Decision: do not cut a public tag under "Oma-" without an explicit OK from the Omarchy project.** The question was written before Quattro made "Oma-" a first-party naming family, and the project already collided once (renamed from `omacalc` on 27 Aug). The legal test is likelihood of confusion with that family, not the prefix itself. Actions: (1) ask — a Discussion on `omacom/omarchy` or the plugin-ecosystem channel is cheap and fast; (2) have a fallback name ready (spec §16.2 already notes `omacell.com` and handles are taken, so brand equity in the name is low); (3) `scripts/rename.sh` (D11) makes this a one-commit change. If the answer is yes, consider asking to publish through the Omarchy Package Repository alongside AUR.

### WP-00.4 ADR-001/002 still proposed
`CLOSED` for ADR-002 (decided by WP-S1). ADR-001 stays Proposed until WP-S2.1/2 pass — see those items.

---

## Phase 1 — engine

### WP-01.1 `ERROR.TYPE` for `#SPILL!` / `#BLOCKED!` / `#CALC!`
`AGENT` · **Decision: use the extended table.** Excel 365: `#NULL!`=1, `#DIV/0!`=2, `#VALUE!`=3, `#REF!`=4, `#NAME?`=5, `#NUM!`=6, `#N/A`=7, `#GETTING_DATA`=8, `#SPILL!`=9, `#CONNECT!`=10, `#BLOCKED!`=11, `#UNKNOWN!`=12, `#FIELD!`=13, `#CALC!`=14, anything else `#N/A`. Microsoft's current ERROR.TYPE page lists all fourteen; the WP-01 agent read an older copy. **H**, one live row in Appendix A.

### WP-01.2 External workbook syntax rejected by `addr.parse`
`CLOSED` — the formula lexer has `ExternalBook` and the AST has `ExprKind::External`, so WP-03 owns it. `addr.parse` (Name Box / Go To input) rejecting `[Book.xlsx]Sheet1!A1` is fine with no multi-workbook engine. Confirm the evaluator treats external refs as L3: cached value on load, never recomputed, never turned into `#REF!` on open (Excel keeps cached values for closed links).

### WP-02.1 Formula rewrite on insert/delete is a no-op in WP-02
`CLOSED` — `core/src/ops.rs` calls `rewrite_formulas` on all four structure operations.

### WP-02.2 Defined-name grammar vs Excel
`AGENT` · **Decision: current rule is right; widen slightly.** Excel rejects any name that parses as a cell reference — `Q1`, `A1`, `XFD1`, `R1C1` — and the bare `R` and `C`. It accepts `\` as a first character and `.`/`?` after the first, up to 255 chars, case-insensitive, no spaces, and rejects `TRUE`/`FALSE`. Add the accept/reject rows to the names corpus. **H**

### WP-02.3 Undo intern-refcount leak on eviction
`AGENT` · **Decision: accept for session lifetime; serialize only live workbook structures.** The XLSX writer already builds its shared-string table by walking live cells, while OMC writes live cells and metadata directly. Regression tests now prove undo-only and evicted-history strings are absent, so a mutating `Workbook::compact_interners()` pass is unnecessary. **L**

### WP-02.4 Row-height / column-width units
`AGENT` · **Decision: `core` stays in pixels; `io` converts at the file boundary; `gui` applies DPI separately.** Excel serialises `ht` in points and `width` in character units of the workbook default font's maximum digit width (MDW): `width = TRUNC((chars·MDW + 5)/MDW·256)/256`, `px = TRUNC(((256·width + TRUNC(128/MDW))/256)·MDW)`. MDW is 7 px for Calibri 11 at 96 DPI. Two rules for G2: (a) keep the original `width`/`ht`/`defaultRowHeight`/`baseColWidth` attributes as L3 extras and re-emit them when the row/column is not dirty, otherwise the round-trip diff will never be empty; (b) depend on `ttf-carlito` (metric-compatible Calibri) so MDW is right on Omarchy — see WP-26.3. **H**

### WP-02.5 Parallel worktree note
`CLOSED` — informational.

### WP-03.1 Cut-paste with a partially overlapping formula range
`AGENT` · **Decision: leave the reference unchanged — matches Excel.** Excel adjusts a reference only when the referenced range lies *entirely* inside the cut area. Corpus row. **H**

### WP-03.2 `XFE1` parses as a name
`AGENT` · **Decision: correct.** Excel's last column is `XFD`; `=XFE1` evaluates to `#NAME?` (an undefined name), not `#REF!`. Corpus row. **M** — live check in Appendix A.

### WP-03.3 Canonical printer upper-cases LAMBDA/LET names in call position
`AGENT` · **Decision: preserve definition-site casing for LET variables and LAMBDA parameters; upper-case only built-in function names.** Excel normalises later uses to the spelling at the definition. **M**

### WP-03.4 `Sheet1!A1:Sheet2!B2` rejected
`AGENT` · **Decision: correct.** Excel rejects a range whose corners carry different sheet qualifiers; the 3-D form is `Sheet1:Sheet2!A1:B2`. **H**

### WP-04.1 Broadcast fills `#N/A`
`AGENT` · **Decision: correct.** `={1,2}+{10,20,30}` → `{11,22,#N/A}`. **H**

### WP-04.2 `AutomaticExceptTables`
`AGENT` · **Decision: What-If data tables (`TABLE()`), as implemented.** Excel's "Automatic except for data tables" has never referred to ListObjects. **H**

### WP-04.3 Legacy multi-cell CSE arrays on import
`AGENT` · **Decision: model them, don't convert them. Implemented 2026-09-01.** Import `<f t="array" ref="A1:B2">` as a per-anchor `ArrayFormula { ref }`: fixed size, formula-bar display `{=…}`, result padded with `#N/A` when smaller and truncated when larger, written back as `t="array"`. Silently converting to dynamic arrays changes semantics and breaks round-trip. The fixed range now lives beside the per-cell flag and is also preserved by OMC. **H**

### WP-04.4 `[[#This Row],[Col]]`
`AGENT` · **Decision: yes — parse as item + column.** It is the long form of `Table1[@Col]`; a column *span* is `[[A]:[B]]`. WP-03 corpus row. **H**

### WP-04.5 Is the 50 ms gate "typical edit" or "any edit in a 100k model"?
`HUMAN` · **Decision (recommended): typical edit.** Define the gate on the p50 fan-out of the G2 real-file corpus and track worst-case (100k direct dependents, currently 228 ms) as a separate metric with a 250 ms budget. Excel itself is visibly slow on that shape. Record in PLAN §12.1. **L**

### WP-05F.1 `SEQUENCE(0)`
`BUG` · **Decision: `#CALC!` for any zero-size array result; `#NUM!` for negative or oversized.** Excel 365 returns `#CALC!` ("Empty Array") for `=SEQUENCE(0)`, and `FILTER` with no matches and no `if_empty`. **H**

### WP-05F.2 Workbook-level locale not on frozen `WorkbookSettings`
`CLOSED` · **Decision: keep locale application-level.** Excel's locale is a system/app setting, not a workbook property; formats are stored locale-independently. No contract change. **H**

### WP-05a.1 `CELL()` without a reference
`CLOSED` · The live `RecalcEngine` retains the session's last changed cell and
uses it for omitted `CELL()` references; direct evaluation without a session
falls back to the formula cell. **H**

### WP-05a.2 `CEILING`/`FLOOR` with opposite signs
`BUG` · **Decision: match Excel 2010+, which is asymmetric.** `CEILING(-2.5, 2)` = `-2` (toward zero) and `FLOOR(-2.5, 2)` = `-4` (away from zero); only a *positive* number with a *negative* significance returns `#NUM!` (`CEILING(2.5,-2)`, `FLOOR(2.5,-2)`). `fn/src/math.rs` currently returns `#NUM!` for `n·sig < 0` in both directions — that is Excel 2007 behaviour. Microsoft's example tables show `=CEILING(-2.5, 2)` → `-2` and `=FLOOR(2.5,-2)` → `#NUM!`. **H**, live rows in Appendix A.

### WP-05a.3 `*IF` family with array constants
`AGENT` · **Decision: strict — `#VALUE!`.** `SUMIF`/`COUNTIF`/`AVERAGEIF(S)` require a range reference; Excel refuses the formula at entry. Since Omacell cannot pop a dialog, `#VALUE!` at evaluation is the closest. Dynamic-array users have `SUMPRODUCT`/`FILTER`. **H**

### WP-05b.1 `CHAR(128)`
`AGENT` · **Decision: Windows-1252.** Windows Excel maps `CHAR(128)` to `€`, `130`→`‚`, `145/146`→`'`/`'` etc.; the five undefined 1252 slots (129, 141, 143, 144, 157) return the C1 control. Files you will open come overwhelmingly from Windows Excel. **H**

### WP-05b.2 `DATEDIF` `"MD"`/`"YD"` at month ends and leap days
`AGENT` · **Decision: reproduce Excel's quirks and document them.** Microsoft's own page warns `"MD"` "may result in a negative number, a zero, or an inaccurate result" — the algorithm is `end.day − start.day`, and when negative adds the length of the month *before* `end`'s month (so `DATEDIF("2019-01-31","2019-03-01","MD")` = `-2`). `"YD"` carries a leap-year off-by-one when `start` is in a leap year. Encode 4–6 pathological pairs as corpus rows marked "Excel bug, reproduced". **M** — live rows in Appendix A.

### WP-05b.3 `YEARFRAC` basis 1
`BUG` · **Decision: Excel's basis-1 is an average-year-length method, not the remaining-days-per-end method described in the report.** `result = (end − start) / D` where: same calendar year → `D` = that year's length; span ≤ one year across a boundary → `D` = 366 if a 29 Feb falls in the span else 365; otherwise `D` = (Jan 1 of `end.year+1` − Jan 1 of `start.year`) / (`end.year − start.year + 1`). Corpus rows for a mixed-leap span. **H**

### WP-05b.4 Show spilled arrays as `{…}` in the UI?
`AGENT` · **Decision: no.** Spilled cells display their values; the formula bar shows the anchor formula (greyed on non-anchor cells). Only legacy CSE arrays (WP-04.3) show `{=…}`. `{…}` stays a corpus-runner rendering. **H**

### WP-05c.1 Duplicate `ISOMITTED` metadata
`CLOSED` — keep it under `lambda`; 05a skips it.

### WP-05c.2 LibreOffice lacks dynamic-array/lambda oracles
`HUMAN` · **Decision: Excel 365 is the oracle for DA/LAMBDA/`XNPV`-class functions.** Re-run the G1 50-row spot check for *those* rows against real Excel (the LibreOffice pass covered the rest). One sitting.

### WP-05c.3 Fill 1M-row criterion wall times
`HUMAN` · run `cargo bench -p omacell-fn --bench lookup_array` on the fixed perf host and commit the baseline.

### WP-06.1 `numFmtId` 14 rendering
`BUG` · **Decision: en-US → `m/d/yyyy`.** ECMA-376 lists id 14 as `mm-dd-yy`, but Excel renders it as the system short date, which is `M/d/yyyy` in en-US — `8/31/2026`, four-digit year. The current `m/d/yy` is wrong for every en-US file. **H**

### WP-06.2 Japanese eras, `[DBNum1]`, Hijri/`B2`
`AGENT` · **Decision: parsed-not-rendered stays; defer past 1.0** unless a G2 file needs them. Render Gregorian/ASCII with a known-difference note. **L**

### WP-06.3 `1.005` with `0.00`
`AGENT` · **Decision: `1.01`.** Excel formats through a 15-significant-digit decimal conversion first ("1.005" exactly), then rounds half away from zero; `=ROUND(1.005,2)` is also `1.01`. Implement display rounding on the 15-digit decimal string, not on the binary double. **H**

### WP-06.4 `-0.0` with a three-section format
`AGENT` · **Decision: zero section — correct.** Sections select on `>0`, `<0`, `=0`, and `-0.0 = 0`. Note the adjacent case: `-0.001` under `0.00;(0.00);"-"` uses the *negative* section and shows `(0.00)`. Corpus rows for both. **H**

### WP-06.5 fr-FR thousands separator
`AGENT` · **Decision: U+202F narrow no-break space.** Excel takes the grouping symbol from the OS; Windows fr-FR has used U+202F since Windows 8. Never U+0020 — it breaks re-parsing and line wrapping. **M**

### WP-07a.1 `edit.undo`/`edit.redo` ids for keymaps
`CLOSED` — confirm `Ctrl+Z`/`Ctrl+Y` bind to them in `default/keys/classic.toml` (Excel default; `F4`/`edit.repeat` is separate).

### WP-07a.2 Should `Ipc` share model-origin policy?
`CLOSED` · **Decision: IPC is a trusted same-user surface** (0700 dir, 0600 socket) and may execute directly; *agents* on IPC must present an agent origin and are changeset-only. The runner-backed dispatcher already refuses `mode=execute` for changeset-eligible mutations — keep that invariant in the single dispatcher (review item 4).

### WP-07a.3 Custom `numFmtId` allocation from 164
`CLOSED` — matches Excel (built-ins occupy 0–163). Confirm WP-09/10 share the table.

### WP-07b.1 / WP-13.2 Hide `edit.undo`/`edit.redo` on `omacell ipc`?
`AGENT` · **Decision: don't hide; scope by origin.** A script driving a live TUI may legitimately undo. Allow for `Origin::Ipc`/`Script`, refuse for agent origins, document. **L**

### WP-07b.2 1 MiB frame / 32 connections
`CLOSED` · **Decision implemented: raise the frame cap to 16 MiB (config-tunable), keep 32 connections.** A 100k-cell `range.set` or `edit.findall` result exceeds 1 MiB as JSON. `[ipc].max_frame_bytes` accepts 1–16 MiB and is enforced by servers, CLI clients, and the Python bridge after restart. Invalid configured/programmatic limits use `ipc.limit`; oversized wire frames preserve the frozen `ipc.frame` classification and add a hint to chunk. **M**

---

## Phase 2 — file I/O

### WP-08.1 Undo unit around CSV `file.open`?
`AGENT` · **Decision: no undo for open; clear the stack.** Excel has no undo for opening a file. **H**

### WP-08.2 Values-mode export writes cached values
`AGENT` · **Decision: `convert` recalculates first unless `calc.mode` is manual or `--no-recalc` is passed.** **H**

### WP-08.3 HTML clipboard entity coverage
`AGENT` · **Decision: full HTML5 named-character table.** Implemented with the existing `quick-xml` HTML entity resolver plus its 94-entry standards supplement (the 92 multi-code-point names and two omitted single-code-point names), with a 64-byte entity-name scan cap. **H**

### WP-09.1 / WP-10.1 Regenerate vs copy worksheet parts
`CLOSED` — decided in WP-10: regenerate modelled parts, copy the rest, inject extras. Record as ADR-007 or in `docs/formats/`.

### WP-09.2 / WP-10.2 Freeze/split units
`BUG` · **Decision: unfrozen splits are in twips (1/20 pt), not pixels.** ECMA-376 `pane`: `xSplit`/`ySplit` are "in 1/20th of a point", or column/row counts when `state="frozen"`. `write.rs` emits `split.x_px`/`split.y_px` and the reader interprets twips as pixels — an Excel split pane opens 15× off in one direction and Excel receives a 15×-too-small position from the other. At 96 DPI, `twips = px × 15`. Fix both sides; add a corpus `.xlsx` with a split pane saved by Excel. **H**

### WP-10.3 VML for notes
`CLOSED` — new notes get minimal VML, source VML preserved. Confirm threaded comments write both `threadedComments` and the legacy `comments` part, as Excel 365 does.

### WP-11.1 Theme/VBA as base64 in `.omc`?
`AGENT` · **Decision: no.** `.omc` exists for readable diffs; document that it is lossy for L3 parts (theme, VBA, drawings) and that `.xlsx` is the fidelity format. Matches F-9.3. **H**

### WP-11.2 Compact vs JSON styles in `.omc`
`AGENT` · **Decision: writer JSON, reader both, for 1.0; deprecate the compact form afterwards** unless the spec needs it. **L**

---

## Phase 3 — surfaces I

### WP-12.1 SIGUSR1 bound only in WP-13
`CLOSED` — signal handling belongs in the process composition root.

### WP-12.2 WP-14 replaces or extends `default/keys/*.toml`?
`CLOSED` — WP-14 owns the keymap engine, `default/keys/` remains shipped data, user overrides layer under `~/.config/omacell/keys/`.

### WP-13.1 Composition root in `crates/cli`
`CLOSED` — `cli` depends on `tui` and `gui`; one binary.

### WP-14.1 Flush `UiSession` freeze/split/zoom into `ViewState` on save
`CLOSED` · Interactive save/save-as flushes retained zoom, panes, selection,
scroll, and formula-display state after the atomic write succeeds. **H**

### WP-14.2 Preserve `KeyOutcome.count`
`CLOSED` · TUI and GUI dispatch preserve modal counts for count-aware commands.

### WP-14.3 Underscore-free command ids in docs
`CLOSED` — `repo_lint` scans `docs/` for underscore-bearing command-id shapes
and distinguishes documented filenames by their known suffix.

### WP-15.1 `[tui] graphics = auto` and chart blitting
`CLOSED` — implemented with a 75 ms bounded terminal query, Kitty/Ghostty
environment hints, shared-scene sixel/Kitty encoding on a bounded worker, and
an ANSI Unicode-braille fallback. tmux/Herdr `auto` falls back; explicit
protocol selection remains available for configured passthrough.

### WP-15.2 Share `App::bootstrap_live`
`CLOSED` — WP-16 uses the shared live bootstrap and task-runner attachment.

### WP-15.3 Manual terminal matrix
`HUMAN` · **Reorder for Quattro:** Foot (default), Ghostty, Kitty, Alacritty, then tmux *and Herdr*, then an SSH session. Part of G3.

### WP-15.4 WP-15a must land first
`CLOSED` — merged.

### WP-15a.1 IPC event subscribe/unsubscribe before G3
`CLOSED` — `Subscribe`/`Unsubscribe` control ops exist in `ipc/protocol.rs`. Exercise them during G3 with `omacell ipc` against a live TUI.

### WP-15a.2 Per-cell stale hatching during recalc
`AGENT` · **Decision: busy chrome only for 1.0.** Excel shows nothing per cell during recalculation, only the status bar. Defer. **L**

---

## Phase 4 — GUI and data tools

### WP-16.1 Multi-window: egui viewports vs one process per file
`HUMAN` · **Decision (recommended): one process per file for 1.0; record as ADR-007.** Quattro launches each app in its own systemd scope and lets `systemd-oomd` kill a runaway process — per-file processes localise a 1M-row OOM to one workbook, and Hyprland tiles each window regardless. IPC discovery already supports multiple instances (`{pid}.sock`, `discover_newest`). Costs: no cross-workbook references (already out of scope) and one engine per process. Revisit viewports after 1.0 if clipboard/coordination pain shows up in dogfooding. **L**

### WP-16.2 CJK IME and Orca outstanding
`HUMAN` · see WP-S2.1/2. This is the G4 item that decides ADR-001.

### WP-16.3 Header sizes via command vs on save
`AGENT` · **Decision: depends what "header" means.** Row heights and column widths are workbook data — they go through `format.rowheight`/`format.colwidth` (undoable, replayable). Row-header width and column-header height are UI chrome Excel does not persist — session state only, never `ViewState`. **M**

### WP-16.4 Cold start 333.67 ms vs 300 ms target
`HUMAN` · **Decision: hold the gate, but measure the real thing first.** The software-adapter number is not the product measurement; WP-28 requires wgpu render/present on the fixed integrated-GPU host. If still over: the usual culprits are wgpu pipeline compilation (persist a pipeline cache under `~/.cache/omacell`) and `fontdb` system scanning (load a bundled fallback for the first frame, scan in a background thread). Quattro's own apps set the bar — Omawrite is described as opening "in a blink". **M**

### WP-17.1 / WP-18.1 RFC approval for frozen-contract changes
`HUMAN` · read the RFC sections, approve or amend, and note the approval in the report. Both packages are already merged, so this is retroactive — do it before the next contract-touching package.

---

## Phase 6 — analysis and output

### WP-25.1 Excel 2016+ chart types (`chartEx`)
`AGENT` · **Decision: L3 preserve, don't model.** Histogram, Pareto, waterfall, treemap, sunburst, box-and-whisker and funnel live in `xl/charts/chartEx*.xml` under the `cx:` namespace, not `c15`. Spec §2.2 already says exotic charts are preserved-not-rendered. Post-1.0. **H**

### WP-25.2 Chart property editing and move/resize
`CLOSED` · WP-28 implements `chart.move`, `chart.resize`, `chart.title`, and
`chart.axistitle` as undoable, changeset-eligible commands reachable through
the retained command surface. The full property panel is post-1.0. **L**

### WP-26.1 Print titles as a count vs `$3:$4` band
`CLOSED` · WP-28 implements explicit start/end row and column bands, XLSX
round-trip, and repeated-band pagination. **H**

### WP-26.2 Native printer dialog vs palette chooser
`CLOSED` · `file.print` opens a retained keyboard/AccessKit printer list from
`lpstat -a` and remembers the last printer. **L**

### WP-26.3 PDF fallback font
`CLOSED` · WP-28 packages Carlito and Liberation, applies Office-compatible
aliases, embeds the resolved face, and documents warning-bearing Standard-14
Helvetica only as the no-font fallback. Aptos remains a documented metric
difference because no free clone exists. **M**

---

## Spikes

### WP-S1.1 ADR-002 decided by merge
`CLOSED`.

### WP-S1.2 Test-only IronCalc oracle
`AGENT` · **Decision: no.** IronCalc evaluates full-sheet and lacks dynamic arrays and LAMBDA, so its oracle value is low next to LibreOffice + openpyxl + Excel-authored fixtures. **L**

### WP-S1.3 Planning-time questions
`CLOSED` — answered in the report.

### WP-S2.1 CJK IME on Wayland — blocks ADR-001
`HUMAN` · G4 procedure: `pacman -S fcitx5 fcitx5-im fcitx5-chinese-addons fcitx5-mozc`; export `GTK_IM_MODULE=fcitx QT_IM_MODULE=fcitx XMODIFIERS=@im=fcitx` in the Hyprland env; egui/winit uses `zwp_text_input_v3`, which Hyprland supports with fcitx5. Type Chinese and Japanese into a cell, into the formula bar, and into the palette. If composition never reaches egui, exhaust winit IME fixes before treating it as Qt-swap trigger 1 — a toolkit swap after four GUI packages is a different project.

### WP-S2.2 Orca speech — blocks ADR-001
`HUMAN` · `pacman -S orca accerciser`; `gsettings set org.gnome.desktop.a11y.applications screen-reader-enabled true`; launch Omacell with the AccessKit feature; Orca must read "cell A1 value …" when the grid is focused and follow arrow keys. Verify the tree in Accerciser first so a failure is attributable.

### WP-S2.3 1.5× hairline is 1–2 px
`AGENT` · **Decision: snap gridlines to physical pixels and force 1 px.** Compute stroke positions in device pixels, round to `n + 0.5`, and never let a 1-logical-px line become 1.5 device px. Verify at 1.25×, 1.5×, 2× during G4. **M**

### WP-S2.4 Product cold start
`CLOSED` — superseded by WP-16.4.

---

## Appendix A — ten-minute live Excel 365 checklist

Type each into a fresh workbook; record the result next to the item and turn it into a corpus row.

| Item | Formula | Expected |
|---|---|---|
| WP-01.1 | `=ERROR.TYPE(SEQUENCE(0))` | 14 |
| WP-01.1 | `=ERROR.TYPE(A1#)` with A1 holding a blocked spill | 9 |
| WP-03.2 | `=XFE1` | `#NAME?` |
| WP-05F.1 | `=SEQUENCE(0)` | `#CALC!` |
| WP-05a.2 | `=CEILING(-2.5,2)` | -2 |
| WP-05a.2 | `=FLOOR(-2.5,2)` | -4 |
| WP-05a.2 | `=CEILING(2.5,-2)` | `#NUM!` |
| WP-05a.3 | `=SUMIF({1,2,3},">1")` | rejected at entry |
| WP-05b.1 | `=CODE(CHAR(128))`, `=CHAR(128)` | 128, `€` |
| WP-05b.2 | `=DATEDIF(DATE(2019,1,31),DATE(2019,3,1),"MD")` | -2 |
| WP-05b.2 | `=DATEDIF(DATE(2020,2,29),DATE(2021,3,1),"YD")` | record |
| WP-05b.3 | `=YEARFRAC(DATE(2019,6,30),DATE(2021,3,15),1)` | record; compare to average-year method |
| WP-06.1 | any date, format id 14 ("Short Date") | `8/31/2026` |
| WP-06.3 | `1.005` formatted `0.00` | `1.01` |
| WP-06.4 | `-0.001` formatted `0.00;(0.00);"-"` | `(0.00)` |
| WP-04.1 | `={1,2}+{10,20,30}` | `{11,22,#N/A}` |
| WP-03.4 | `=SUM(Sheet1!A1:Sheet2!B2)` | rejected at entry |

## Appendix B — things not in the 83 that Quattro surfaced

1. **Resolved by WP-28:** setup links the skill into all nine generic/harness-specific locations, including OpenCode, Grok, Copilot, and Crush.
2. Hand-off cwd is the workbook directory; Omarchy starts agents in `~/Work` for trust persistence. Decide at G5.
3. **Resolved by WP-28:** `HYPRLAND_SNIPPET` uses Quattro's `{ launch = "omacell" }` table form.
4. **Resolved by WP-28:** the expanded semantic theme keys feed the GUI role map.
5. **Resolved by WP-28:** Arch packages depend on `ttf-carlito` and `ttf-liberation`.
6. Distribution: Omacalc, NordVPN and others ship through the Omarchy Package Repository. Ask about it in the same conversation as WP-00.3.

---

## Implementation follow-up

The repository owner supplied this triage and requested the applicable fixes on
31 August 2026. The follow-up implements the decisions that correct existing,
already-shipped behavior:

- Excel 365 `ERROR.TYPE` codes 9–14 and defined-name grammar;
- zero-sized `SEQUENCE`, legacy `CEILING`/`FLOOR`, Windows-1252 `CHAR`/`CODE`,
  and `YEARFRAC` basis 1 semantics;
- en-US built-in short dates, 15-digit decimal display rounding, and the
  fr-FR narrow no-break grouping separator; and
- OOXML split-pane twip conversion at the file boundary.

This changes behavior behind frozen public functions, but does not change any
Rust type, serialized enum, command schema, or IPC wire shape. The owner-provided
decisions and explicit request to apply required fixes are the human approval
for those semantic corrections. Items assigned to later packages or to live
hardware/product checks remain deferred as labeled above. The remaining
non-`BUG` enhancements stay with their named future package or a separately
scoped follow-up so this correction does not silently expand frozen command,
configuration, or IPC contracts.
