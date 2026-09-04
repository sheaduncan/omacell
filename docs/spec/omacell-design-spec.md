# Omacell — Design Specification

**A spreadsheet for Omarchy Linux**

| | |
|---|---|
| Version | 0.3 — draft for review |
| Change log | 0.1 initial draft · 0.2 adds the AI-native design (§8) and integrates it across goals, principles, functional spec, tunability, architecture, security, testing, roadmap, and appendices · 0.3 renames the working name from Omacalc to Omacell throughout (binary `omacell`, config under `~/.config/omacell/`, crates `omacell-*`) |
| Date | 27 August 2026 |
| Owner | Shea Duncan |
| Status | Draft |
| Target platform | Omarchy 4.x "Quattro" (Arch Linux · Hyprland · Quickshell) |
| Working name | *Omacell* (chosen 27 Aug 2026; previously *Omacalc*). The "Oma-" prefix implies membership in the Omakase family (Omakub, Omarchy, Omacom) and should be cleared with that project before shipping; run a trademark clearance search before the first public tag. |

---

## 1. Summary

Omacell is a spreadsheet application built for Omarchy. It keeps the *semantics* of Excel — the grid, A1 references, the formula language, automatic recalculation, number formats, the error model, the keyboard vocabulary, and `.xlsx` as the exchange format — and discards the *ceremony*: ribbons, floating dialogs, an in-app theme system, a settings maze. In their place it adopts Omarchy's contract with the user: everything is a text file in `~/.config`, the active Omarchy theme is the only theme, the keyboard is the primary input, the terminal is a first-class client, and nothing phones home. It is also AI-native in Omarchy's sense of the word: models and agents are first-class operators of the grid, no vendor is chosen for the user, and nothing runs or leaves the machine until the user configures it.

The product is one engine with three front-ends: a native Wayland GUI, a terminal UI, and a JSON-speaking command line that doubles as an IPC surface for scripts and AI agents. A file opened in any of them is the same file with the same recalculation results. Models act on workbooks the same way the front-ends do — through the command bus — so every AI action is a reviewable, undoable changeset.

The document is organized as follows: goals and non-goals (§2), the two bodies of principle being reconciled (§3), the design principles that reconcile them (§4), product definition (§5), functional specification (§6), Omarchy integration (§7), the AI-native design (§8), the tunability model (§9), user experience (§10), architecture (§11), non-functional requirements (§12), an Excel compatibility matrix (§13), quality strategy (§14), roadmap (§15), risks and open decisions (§16), and appendices with the default keymap, sample configuration, theme template, function tiers, and file format sketch.

## 2. Goals and non-goals

### 2.1 Goals

1. **Be a real spreadsheet.** Formulas, references, recalculation, formats, sorting, filtering, tables, validation, conditional formatting, pivots, and charts — the things people open Excel for — work the way an Excel user expects.
2. **Round-trip `.xlsx` faithfully.** A file received from an Excel user can be opened, edited, saved, and returned without surprising them.
3. **Feel native to Omarchy.** Adopts the active theme automatically, follows the system font and text size, tiles cleanly under Hyprland, is driven from the keyboard, and is configured through text files that survive updates and reset with one command.
4. **Be tunable at every layer.** Every visual and behavioral property has a documented key, a documented default, a documented precedence, and — where practical — hot reload.
5. **Serve the terminal.** A TUI for working over SSH or inside a tmux pane, and a CLI for conversion, querying, and scripted recalculation.
6. **Stay local.** No network access by default, no telemetry, no accounts. Files are plain artifacts on disk.
7. **Be scriptable and agent-friendly.** Lua in-process for users; a JSON command surface, an MCP server, and a shipped agent skill so any tool or coding agent can operate a workbook, in the same spirit as the `omarchy` CLI.
8. **Be AI-native without picking a vendor.** Models help in cells, in the formula bar, in the palette, on import, and in an agent panel; local models are the default path; every AI action is a reviewable, undoable changeset; nothing runs until the user configures it (§8).

### 2.2 Non-goals (v1)

- Real-time multi-user collaboration.
- Executing VBA. Files containing VBA are opened with the macro payload preserved but inert.
- Parity with Power Query, Power Pivot, or the data-model engine.
- Full parity with Excel's charting engine. Core chart types round-trip; exotic ones are preserved-but-not-rendered.
- Legacy binary `.xls` write support (read is native; see §6.9).
- A ribbon, an in-app theme editor, or a settings GUI. Configuration is text.
- Bundling a model, a vendor account, or a default AI provider. AI features stay off until the user configures a provider or picks an Omarchy default agent.
- AI that acts unprompted: background analysis, suggestions on open, or "smart" silent conversions.
- Training on or uploading user data. Requests contain only what the configured privacy level allows, and only when a user or agent action triggers them.
- Windows or macOS builds. Portability to other Wayland desktops is a design constraint, not a release target.

## 3. Background

### 3.1 What Excel is, distilled

Excel's durability comes from a small set of ideas that compound. The following list is the definition of "carrying over the principles of what Excel is" used throughout this document.

1. **The grid is the program.** A sheet is a live dataflow program that non-programmers can write. Each cell holds either a literal or a formula; the sheet as a whole is the computation.
2. **Direct manipulation, immediate feedback.** Type, see the result, adjust. There is no "run" step.
3. **Formulas are referential and copyable.** The relative/absolute reference model (`A1`, `$A$1`, `A$1`) plus fill operations make patterns cheap to express: write once, fill down.
4. **Values are separate from presentation.** `0.25` displayed as `25%` is still `0.25`. Dates are numbers with a format. Formatting never changes what a formula sees.
5. **Errors are values.** `#DIV/0!`, `#N/A`, `#REF!`, `#VALUE!`, `#NAME?`, `#NUM!`, `#SPILL!`, `#CALC!` propagate through dependents rather than halting the sheet, and can be handled (`IFERROR`, `IFNA`).
6. **Automatic recalculation over a dependency graph.** The user never manages evaluation order.
7. **Data tools live next to the data.** Sort, filter, validate, conditionally format, pivot, and chart happen on the same grid, not in a separate tool.
8. **Many sheets, one portable document.** Cross-sheet references, defined names, and tables make a workbook a self-contained model that can be emailed.
9. **Keyboard fluency compounds.** Expert use is keyboard-driven: `Ctrl+Arrow`, `F2`, `F4`, `Ctrl+D`, `Ctrl+Shift+L`, `Alt+=`. The vocabulary is a shared professional dialect.
10. **Extensibility and interoperability.** `LAMBDA`/`LET`, macros, and add-ins extend the language; `.xlsx` (OOXML) is the lingua franca of tabular exchange.

Two Excel lessons are also carried over as *anti-patterns*: silent type coercion on import (gene names becoming dates) and macros executing on open. Omacell's import preview and script trust model exist because of them.

### 3.2 What Omarchy is, distilled (as of 4.0 "Quattro", August 2026)

Omarchy is DHH's opinionated Arch Linux distribution: Hyprland as compositor, and — since 4.0, tagged 14 August 2026 — a single Quickshell (Qt Quick) process for the bar, launcher, menus, notifications, OSD, and lock screen. The properties that shape this specification:

| Property | Consequence for Omacell |
|---|---|
| **Themes are data.** A theme is a directory whose core is `colors.toml`; Omarchy generates per-app configs from `.tpl` templates on every theme switch, and users can add templates for apps Omarchy does not cover in `~/.config/omarchy/themed/`. 22 themes ship; more install from git. | Omacell ships a template and a color-role mapping; it never ships a theme of its own. |
| **Dotfiles are the interface.** `~/.config` belongs to the user and is never overwritten by updates; `/usr/share/omarchy` belongs to the package. `omarchy reinstall configs` resets. | Same split: `/usr/share/omacell/default/` vs `~/.config/omacell/`; `omacell config reset`. |
| **Hooks and a CLI.** `~/.config/omarchy/hooks/theme-set.d/`, `font-set.d/`, etc. run scripts on events. The `omarchy` command center exposes every action, with `--json` output, explicitly so AI agents can drive it. | Omacell installs a `theme-set` hook, and its own CLI mirrors the `omarchy <group> <command>` shape with `--json`. |
| **Keyboard first, Super is the compositor's.** `Super+Space` is the Omarchy menu, `Super+Alt+Space` the app launcher, `Super+Ctrl+Shift+Space` the theme picker. Hyprland bindings are Lua (`~/.config/hypr/bindings.lua`). | App bindings avoid `Super`; every action is reachable from the keyboard; launch bindings are documented as Lua snippets. |
| **Monospace everywhere.** JetBrainsMono Nerd Font is the default terminal *and* system font; *Style > Font* changes it everywhere; a display text-size knob (9–20 px) moves shell font, GTK text scaling, and terminal point size together. | The UI chrome follows the system monospace font and text size; cell fonts default to it and honor per-cell fonts from files. |
| **Terminal-heavy, TUI-delighted.** Foot is the default terminal; Neovim the default editor; the manual has a page for TUIs. | A terminal front-end is a launch requirement, not a stretch goal. |
| **Shell is themeable and scriptable.** `shell.toml` carries surface roles, control states, spacing, typography, corner radius; `~/.config/omarchy/shell.toml` is a machine-level override; the shell has a plugin system and is IPC-scriptable. | Omacell reads the same spacing/typography/corner tokens so it looks like part of the shell, not a guest. |
| **Calc coexistence.** LibreOffice may be installed as a second spreadsheet. | Omacell does not need to out-feature Calc; it needs to out-*fit* it. Omacell does not require Calc to open supported formats. |
| **Everything is a package.** Omarchy itself ships as Arch packages; `omarchy-pkg-add` installs from the repos/AUR. | Distribute as an Arch package (AUR first), never by curl-piping or by writing into Omarchy's directories. |
| **AI agents are first-class, but no favorite is picked.** Ten coding-agent CLIs (`claude`, `codex`, `opencode`, `pi`, …) ship as lazy launchers; the user chooses a default agent (`omarchy default agent`) or doesn't, and agentic features stay off until they do. A shipped skill teaches agents to tailor the system; crash diagnosis hands core dumps to the default agent; LM Studio and Ollama are the recommended local-LLM paths; `omarchy-notification-send` is the notification convention. | Omacell's AI layer is off by default, vendor-neutral, works with the user's default agent through a skill and an MCP server, treats local models as the primary path, and hands hard problems (`#REF!` cascades, circular references) to the agent the way Omarchy hands off crashes (§8). |

Note on churn: 4.0 moved the active-theme location from `~/.config/omarchy/current/theme` (v3) to `~/.local/state/omarchy/current/theme`. Omacell resolves both (§7.1) and treats the Omarchy integration layer as something to re-verify on every Omarchy release.

### 3.3 Prior art

| Project | What to learn from it | Why it is not the answer |
|---|---|---|
| **LibreOffice Calc** | Breadth; ODS; a reference implementation for function semantics (usable headless for cross-checking). | Heavy, its own widget toolkit, mouse-and-menu oriented, not theme-aware in Omarchy's sense. |
| **Gnumeric** | Rigorous statistical functions; small and fast. | GTK-classic UI; dormant xlsx fidelity; not keyboard-modal. |
| **sc-im** | The Vim-spreadsheet keyboard grammar (`dr`, `yr`, `:sort`, `v` ranges) — the best existing model for a modal keymap. | Terminal only; thin xlsx support; no styles/pivots/charts. |
| **VisiData** | Column-typed, keyboard-driven data exploration. | An explorer, not a spreadsheet: no cell formulas. |
| **IronCalc** (Rust) | An open-source, xlsx-aware spreadsheet engine with an Excel-compatible formula model. Candidate for the core rather than building from zero (ADR-002). | Engine only; UI and Omarchy fit are ours to build. |
| **Quadratic** | GPU-rendered infinite grid; code cells. | Cloud-first, browser-first; not the file model Excel users need. |
| **Excel, Google Sheets, Grist** | Dynamic arrays, `LET`/`LAMBDA`, structured references; Grist's column-typed tables. | Not local, not themeable, not ours. |

## 4. Design principles

These ten rules resolve conflicts between §3.1 and §3.2. When a later section is silent, apply these.

1. **Excel semantics, Omarchy skin.** Compatibility of *meaning* is non-negotiable: A1 references, formula grammar, function names and results, error values, number-format codes, date serials, the classic keyboard vocabulary, and `.xlsx` round-tripping. Chrome, ceremony, and defaults are ours to change.
2. **Text is the interface.** Every setting lives in a text file under `~/.config/omacell/`. Anything that can be clicked can also be configured, scripted, or invoked from the CLI. There is no setting that exists only in a dialog.
3. **Layered configuration, sane defaults, one-command reset.** Package defaults → active theme → user files → workbook → environment → flags. `omacell config reset` returns to defaults with a backup, exactly as `omarchy reinstall configs` does.
4. **One theme: the active one.** Omacell has no themes of its own. It derives every color from the active Omarchy theme, reacts to theme switches live, and guarantees readable contrast for the derived roles.
5. **Keyboard-first, mouse-complete.** Every action has a key. The mouse is never required and never blocked. Two keymaps ship — Excel-classic (default) and Vim-modal — and both are remappable.
6. **Tile well.** No minimum-size assumptions; usable in a third of a 1080p screen. Dialogs are in-window panels, not floating windows. State (open files, cursor, scroll, panels) survives restarts.
7. **One engine, three clients.** GUI, TUI, and CLI/IPC share one core and one command bus. A command is a name plus JSON arguments; the front-ends are thin.
8. **Local-first, no network by default.** No telemetry. Web functions, external links, and script I/O are off until the user turns them on, per workbook or globally.
9. **Fast at real sizes.** A million-row CSV and a hundred-thousand-formula model are the working assumptions, not stress tests.
10. **Scriptable by people and agents.** Lua in-process (the Omarchy/Neovim/Hyprland dialect), and a JSON command surface with `--json` everywhere, so an agent can open, query, edit, recalc, and export a workbook without a screen.
11. **AI-native, vendor-neutral, off until chosen.** Models are first-class operators of the grid — but they act only through the command bus as reviewable changesets, they see the workbook through a redactable context object, local models are the default path, and no model or vendor is chosen for the user (§8).

## 5. Product definition

### 5.1 Users

- **The Omarchy resident.** A developer or technical professional who lives in the terminal, has Neovim muscle memory, and needs a spreadsheet for budgets, engineering calcs, CSV wrangling, and models — without a context switch into a foreign-looking app.
- **The `.xlsx` recipient.** Anyone who receives workbooks from Excel users and must open, edit, and return them with formulas, formats, and structure intact.
- **The automator.** Someone who wants `omacell convert`, `omacell query --json`, and `omacell recalc` in a shell pipeline, cron job, or agent workflow.
- **The agent operator.** Someone whose default Omarchy agent already does their coding and who expects `omarchy agent prompt "reconcile these two sheets"` to just work — or who wants a model in a cell.

### 5.2 Primary use cases

1. Open a colleague's `.xlsx`, fix numbers, add a formula column, save, send back.
2. Import a 1M-row CSV export, type columns correctly on import, filter and pivot it, chart the result.
3. Build a financial or engineering model with named ranges, `LET`/`LAMBDA`, data validation, and Goal Seek.
4. Work over SSH in the TUI on a server-side CSV or workbook.
5. Drive the app from a script or agent: `omacell set model.xlsx Inputs!B3 0.07 && omacell recalc model.xlsx --write && omacell query model.xlsx Summary!A1:D20 --json`.
6. Ask in plain language — "sort by Amount, flag totals over 10k, add a month column from the date" — review the plan, apply it as one undoable change.
7. Hand a messy 40-sheet workbook to the default agent to audit for broken references and hard-coded constants, then review its proposed changeset in the grid.

### 5.3 Target workloads (design assumptions)

| Workload | Assumption |
|---|---|
| Grid size | Excel limits: 1,048,576 rows × 16,384 columns per sheet (kept for `.xlsx` representability). |
| Populated cells | Up to ~20M per workbook in memory; ~2M with formulas. |
| Files | CSV to 1 GB (progressive load), `.xlsx` to 200 MB. |
| Hardware | A current Framework 13 / Dell XPS class laptop on integrated graphics; a 4-year-old laptop must remain usable. |

### 5.4 Scope by front-end

| Capability | GUI | TUI | CLI/IPC |
|---|---|---|---|
| Edit cells, formulas, navigation | ● | ● | ● (by command) |
| Styles, number formats, conditional formatting | ● | ◐ (attributes rendered as terminal styles) | ● |
| Sort, filter, tables, validation | ● | ● | ● |
| Pivot tables | ● | ● (tabular layout) | ● |
| Charts | ● | ◐ (sparkline/bar approximations) | export only |
| Comments, hyperlinks, protection | ● | ● | ● |
| Print / PDF | ● | — | ● |
| Theme following | ● | ● (via terminal palette) | n/a |
| AI features (§8) | ● | ● (agent panel as a pane; no `render` tool) | ● (functions via recalc; MCP; hand-off) |

● full · ◐ partial · — not applicable

## 6. Functional specification

Requirement identifiers (`F-x.y`) are stable for tracking. "MUST/SHOULD/MAY" carry their RFC 2119 meanings.

### 6.1 Workbook and sheet model

- **F-1.1** A workbook contains one or more ordered sheets. Sheets have a name (Excel naming rules: ≤31 chars, no `[]:*?/\`), visibility (visible/hidden/very-hidden), tab color, protection state, and view state (zoom, frozen panes, split, scroll position, selection, gridlines on/off, show-formulas).
- **F-1.2** Sheet dimensions are Excel's: rows 1–1,048,576, columns A–XFD. Attempts to address beyond return `#REF!`.
- **F-1.3** Defined names exist at workbook and sheet scope, may refer to ranges, constants, or formulas, and are usable in any formula (`=Revenue*TaxRate`).
- **F-1.4** Tables (structured ranges) have a name, header row, optional totals row, banded style, and auto-expand on adjacent entry. Structured references (`Sales[Amount]`, `Sales[@Amount]`, `Sales[[#Headers],[Amount]]`) are supported in formulas.
- **F-1.5** External workbook links are preserved on load, displayed with cached values, and never followed unless the user enables `[files] follow_external_links` (default `false`).
- **F-1.6** Workbook metadata (title, author, custom properties), calculation settings (mode, iteration, precision), and the 1900/1904 date system are stored per workbook.

### 6.2 Cell model

- **F-2.1 Value types.** Empty, Number (IEEE 754 double), Text (UTF-8), Boolean, Error, and Array (for spilled results). Dates and times are Numbers with a date/time number format; the 1900 system reproduces Excel's Lotus leap-year quirk (serial 60 = 29 Feb 1900) for compatibility.
- **F-2.2 Cell record.** Input (literal text or formula source), cached value, number format, style reference, optional note, optional threaded comment, optional hyperlink, optional validation rule reference, protection flags (locked/hidden).
- **F-2.3 Number formats.** Excel format-code syntax is implemented in full: sections (`pos;neg;zero;text`), conditions (`[>=1000]`), colors (`[Red]`), literals, scaling (`,`), percent, scientific, fractions, dates/times with elapsed forms (`[h]:mm`), text placeholder `@`, locale codes (`[$-409]`), and the `General` algorithm (up to 11 significant digits, scientific fallback).
- **F-2.4 Styles.** Font (family, size, bold/italic/underline/strike, color), fill (solid, pattern, gradient preserved), borders (per side, style, color), alignment (horizontal, vertical, wrap, shrink, indent, rotation), and protection. Styles are interned records shared across cells, mirroring the `.xlsx` style table so round-tripping is lossless.
- **F-2.5 Rich text.** Runs with per-run font attributes within a cell are preserved and editable.
- **F-2.6 Display precision.** Numbers display with Excel's 15-significant-digit rule; `precision_as_displayed` is available per workbook and off by default.

### 6.3 Formula language and engine

- **F-3.1 Grammar.** Excel's operator set and precedence: `:` (range), space (intersection), `,` (union), unary `-`, `%`, `^`, `*` `/`, `+` `-`, `&`, comparison (`=`, `<>`, `<`, `<=`, `>`, `>=`). Array constants `{1,2;3,4}`. Parenthesized nesting, function calls with omitted arguments (`INDEX(A1:C9,,2)`), and up to 8,192 characters per formula.
- **F-3.2 References.** A1 and R1C1 notations (display switchable; storage is canonical). Relative/absolute mixes. Sheet-qualified (`'Cost Model'!B2`), 3-D (`Sheet1:Sheet3!A1`), whole-row/column (`3:3`, `B:B`), structured, spill (`A1#`), implicit-intersection operator `@`. Copy/paste and fill adjust relative references; `F4` cycles anchoring.
- **F-3.3 Dynamic arrays.** Any formula may return an array; results spill into adjacent empty cells. Blocked spills produce `#SPILL!` with the blocking cell identified. Legacy CSE array formulas from files are preserved and evaluated.
- **F-3.4 Named-lambda constructs.** `LET`, `LAMBDA`, and the lambda helpers (`MAP`, `REDUCE`, `SCAN`, `BYROW`, `BYCOL`, `MAKEARRAY`, `ISOMITTED`) are supported, including lambdas stored in defined names.
- **F-3.5 Coercion and comparison.** Excel rules: empty → 0 or `""` by context; `TRUE`/`FALSE` → 1/0 in arithmetic; numeric text coerces in arithmetic but not in comparison; text comparison is case-insensitive; errors propagate in evaluation order.
- **F-3.6 Recalculation.** A dependency graph with range-aware edges drives incremental recalculation. Modes: automatic (default), automatic-except-tables, manual (`F9`). Volatile functions (`NOW`, `TODAY`, `RAND`, `RANDBETWEEN`, `RANDARRAY`, `OFFSET`, `INDIRECT`, `INFO`, `CELL`) recalculate on every pass. Circular references are detected and reported in the status line; iterative calculation (max iterations, max change) is a per-workbook option.
- **F-3.7 Parallelism.** Independent subgraphs evaluate concurrently. Results MUST be deterministic regardless of thread count.
- **F-3.8 Auditing.** Show formulas mode, trace precedents/dependents (highlighting plus a navigable list), Evaluate Formula step-through, and error explanations (`#NAME?` names the unknown token; `#REF!` names the deleted range).

### 6.4 Function library

Functions are named and behave as in Excel; English canonical names are always accepted. Coverage is tiered (full list in Appendix D):

- **Tier 0 (v1, ~260 functions):** math and trig; statistics (descriptive, `RANK`, `PERCENTILE.INC/EXC`, `QUARTILE`, `MODE`, `FORECAST.LINEAR`); text (`TEXT`, `TEXTJOIN`, `TEXTSPLIT`, `TEXTBEFORE/AFTER`, `REGEX*` as an extension, `SUBSTITUTE`, `MID`, `LEN`, unicode-correct); logical (`IF`, `IFS`, `SWITCH`, `AND`, `OR`, `XOR`, `NOT`, `IFERROR`, `IFNA`); lookup and reference (`XLOOKUP`, `XMATCH`, `INDEX`, `MATCH`, `VLOOKUP`, `HLOOKUP`, `FILTER`, `SORT`, `SORTBY`, `UNIQUE`, `SEQUENCE`, `CHOOSE`, `OFFSET`, `INDIRECT`, `ROW(S)`, `COLUMN(S)`, `TAKE`, `DROP`, `CHOOSEROWS/COLS`, `VSTACK`, `HSTACK`, `TOCOL`, `TOROW`, `WRAPROWS/COLS`); date and time (`DATE`, `EDATE`, `EOMONTH`, `NETWORKDAYS.INTL`, `WORKDAY.INTL`, `DATEDIF`, `YEARFRAC`, `WEEKNUM`, `ISOWEEKNUM`); financial core (`PMT`, `IPMT`, `PPMT`, `NPV`, `XNPV`, `IRR`, `XIRR`, `MIRR`, `FV`, `PV`, `RATE`, `NPER`, `SLN`, `DB`, `DDB`); information (`IS*`, `TYPE`, `ERROR.TYPE`, `NA`, `CELL` subset); aggregation (`SUMIF(S)`, `COUNTIF(S)`, `AVERAGEIF(S)`, `MAXIFS`, `MINIFS`, `SUMPRODUCT`, `AGGREGATE`, `SUBTOTAL`).
- **Tier 1 (v1.x, ~200):** full statistical distributions (`NORM.*`, `T.*`, `CHISQ.*`, `F.*`, `BINOM.*`, `POISSON.*`, `WEIBULL.DIST`, `LOGNORM.*`, `GAMMA.*`), regression (`LINEST`, `LOGEST`, `TREND`, `GROWTH`), engineering (`CONVERT`, `BESSEL*`, `COMPLEX`/`IM*`, `BIN2*`/`HEX2*`/`DEC2*`, `ERF`, `GAMMALN`), database (`DSUM` family), remaining financial (bonds, T-bills, `DURATION`), `FORECAST.ETS` family.
- **Tier 2 (off by default, network policy):** `WEBSERVICE`, `FILTERXML`, `ENCODEURL`, `IMAGE`, RTD.
- **AI namespace:** `AI`, `AI.EXTRACT`, `AI.CLASSIFY`, `AI.FILL`, `AI.TABLE`, `AI.TRANSLATE` — specified in §8.3. They are live only when a provider is configured; otherwise they evaluate to `#N/A` with a hint and keep any cached value.
- **Extension namespace:** user and plugin functions are registered under a namespace (`=MY.WEIBULL_B(range)`), behave as first-class (typed arguments, array-aware, participate in the dependency graph), and are visible to autocomplete and `omacell fn list --json`.

### 6.5 Editing, navigation, and selection

- **F-5.1 Two keymaps.** *Classic* (default) reproduces Excel's keyboard vocabulary (Appendix A). *Modal* is a Vim-style layer modeled on sc-im: Normal/Insert/Visual/Command modes, `hjkl`, counts, operators (`d`, `y`, `p`), `:` commands, `/` search. Selected in `keys.toml` (`model = "classic" | "modal"`); either can be extended or overridden per key.
- **F-5.2 Edit modes.** In-cell editing (`F2` / `i`) and formula-bar editing behave identically. During formula entry, arrow keys and mouse clicks insert references ("point mode"); references are colorized and their ranges outlined in matching colors (theme-derived cycle).
- **F-5.3 Entry behavior.** `Enter` commits and moves (direction configurable, default down); `Tab` moves right; `Ctrl+Enter` fills the selection; `Alt+Enter` inserts a line break; `Esc` cancels. Autocomplete offers function names with signature hints, defined names, table columns, and (for text) matching values from the same column.
- **F-5.4 Selection.** Single, rectangular, multi-area (Ctrl-click / union), whole row/column, current region (`Ctrl+A`, `Ctrl+*`), extend mode (`F8`). Selection statistics (sum/count/avg/min/max, configurable) show in the status line.
- **F-5.5 Fill and series.** Fill handle (mouse) and `Ctrl+D`/`Ctrl+R`/`Ctrl+E` (Flash Fill, tier 1). Linear, growth, date, weekday, month, year, and custom-list series. Fill options (copy vs series vs formats only) exposed as a post-fill choice.
- **F-5.6 Clipboard.** Internal copy preserves everything; the Wayland clipboard offers `text/plain` (TSV), `text/html` (table), `text/csv`, and `text/markdown` (developer convenience) plus an internal MIME type. Paste Special: values, formulas, formats, transpose, skip blanks, operation (add/subtract/multiply/divide), paste link. Cut/paste moves references like Excel.
- **F-5.7 Undo/redo.** Unbounded within a memory budget; grouped by user action; survives sheet switches; a visual undo history panel lists actions.
- **F-5.8 Find/Replace/Go To.** Find in values or formulas, whole-cell, case, regex (extension), scope sheet/workbook; replace with preview count. Go To by address/name; Go To Special (blanks, constants, formulas by result type, errors, visible only, precedents/dependents, conditional formats, validation).
- **F-5.9 Mouse.** Click/drag select, double-click edit, header click select, header-border double-click auto-fit, drag-move ranges (with `Ctrl` copy), fill handle, context menu, `Ctrl+scroll` zoom, horizontal scroll. Trackpad gestures per Hyprland input settings.

### 6.6 Data tools

- **F-6.1 Sort.** Multi-key (values, cell color, font color, icon), ascending/descending, custom lists, case sensitivity, header detection, sort left-to-right. Sorting within tables and filtered ranges behaves as in Excel (hidden rows untouched).
- **F-6.2 AutoFilter.** Per-column value checklists with search, text/number/date filters (contains, between, top N, above average, date periods), color filters, filter state saved in the file, `Ctrl+Shift+L` toggle, clear-all.
- **F-6.3 Tables.** Create (`Ctrl+T`), resize, convert to range, banded styles derived from the theme, totals row with function chooser, calculated columns auto-fill, slicers (tier 1).
- **F-6.4 Data validation.** Whole number, decimal, list (inline or range, with dropdown), date, time, text length, custom formula; input message; error style stop/warning/information; circle-invalid-data view.
- **F-6.5 Conditional formatting.** Cell-value, formula, text, date, blanks/errors, duplicates/uniques, top/bottom N or %, above/below average, color scales (2/3-color), data bars (solid/gradient, negative axis), icon sets. Rule precedence and stop-if-true. Rules are edited in a panel with live preview and stored in `.xlsx`-native form.
- **F-6.6 Structure.** Insert/delete cells, rows, columns (shift semantics as Excel), hide/unhide, group/outline with levels and subtotals, freeze panes, split panes, row heights and column widths (including auto-fit), merge/center and merge-across, cell text wrap.
- **F-6.7 Text to Columns / Remove Duplicates / Consolidate.** Delimited and fixed-width splitting with per-column types; duplicate removal by selected columns with a count report; consolidate by position or category (tier 1).
- **F-6.8 Comments and notes.** Legacy notes and threaded comments both read/write; a comments panel lists, navigates, resolves.
- **F-6.9 Hyperlinks.** Cell links to URLs, files, and in-workbook locations; `xdg-open` for external targets; internal targets navigate.
- **F-6.10 Protection.** Sheet and workbook protection with Excel-compatible password hashing (documented as *not* a security feature), lock/hide flags, allowed actions list, protected-range editing lists (preserved).

### 6.7 Analysis

- **F-7.1 Pivot tables.** Row/column/value/filter fields; SUM, COUNT, AVERAGE, MIN, MAX, COUNTA, DISTINCT COUNT, STDEV, VAR; show values as % of total/row/column, running total, difference from; grouping by dates (days/months/quarters/years) and numeric bins; compact/outline/tabular layouts; subtotals and grand totals; refresh on source change (manual or on open); pivot charts. Storage: native representation plus `.xlsx` pivot cache/definition on export so Excel treats it as a live pivot (fidelity level 2; see §6.9).
- **F-7.2 What-if.** Goal Seek (v1), Data Tables one- and two-variable (v1.x), Scenario Manager (v1.x). Solver is a plugin (later).
- **F-7.3 Statistics panel.** Quick descriptive statistics and a histogram for any selection (native; exportable to cells).

### 6.8 Charts

- **F-8.1 Types (v1).** Line, column/bar (clustered, stacked, 100%), area, pie/donut, scatter, bubble, combo (secondary axis), histogram, and in-cell sparklines (line, column, win/loss).
- **F-8.2 Model.** Charts reference ranges and update on recalc. Series, axes, titles, legends, data labels, gridlines, trendlines (linear, exponential, moving average), error bars (tier 1).
- **F-8.3 Appearance.** Default palette and chrome derive from the theme (Appendix C); per-chart overrides are stored. Charts render identically in the GUI and in PDF/SVG/PNG export.
- **F-8.4 Interoperability.** Core types read and write `.xlsx` DrawingML. Unsupported chart types are preserved as opaque parts and shown as a placeholder with the chart's title and source ranges.

### 6.9 File formats and interoperability

- **F-9.1 Default save format is `.xlsx`.** Rationale: zero lock-in, maximum interchange, and the file already has a place for everything Excel needs. App-specific extras (modal-key settings, view layouts, Lua scripts, AI result cache and provenance — §8.3) are written to a namespaced custom part (`xl/omacell/`) that Excel ignores; nothing essential depends on Excel preserving it.
- **F-9.2 Fidelity levels for `.xlsx`.** L1 — cell values, formulas, and number formats round-trip losslessly. L2 — styles, merged cells, defined names, tables, validation, conditional formatting, comments, hyperlinks, freeze/split, print settings, pivot definitions, core charts. L3 — unknown parts (VBA, custom XML, exotic drawings, form controls, embedded objects) are preserved byte-for-byte and re-emitted. The test corpus measures each level (§14).
- **F-9.3 Text workbook (`.omc`).** A documented, line-oriented plain-text format (Appendix E) for git-friendly diffs, hand editing, and generation by scripts. It carries everything `.xlsx` L1–L2 carries except binary parts.
- **F-9.4 CSV/TSV.** Import with an interactive preview: delimiter, quoting, encoding (UTF-8 with BOM detection, Latin-1, UTF-16), decimal and thousands separators, header row, per-column type (auto/number/text/date with format/boolean), and "keep as text" for ambiguous columns. No silent conversion; the preview shows what would change. Export with the same controls plus line endings. Files over the in-memory threshold load progressively with a visible row count.
- **F-9.5 Other formats.** ODS read (v1) and write (v1.x); JSON (array-of-objects ↔ table, nested flattening rules); Parquet/Arrow read (v1.x); HTML and Markdown tables via clipboard and import; legacy `.xls` read in-process with a bounded BIFF parser. `.xls` write is never supported.
- **F-9.6 Safety.** Zip and XML readers enforce size and expansion limits, disable external entities, and are fuzzed. Files never execute embedded scripts on open (§12.3).
- **F-9.7 Locking and autosave.** Cooperative lock files compatible with LibreOffice's `.~lock.<name>#` convention so Calc and Omacell warn each other. Autosave to `~/.local/state/omacell/autosave/` on an interval, with crash recovery on next launch. Optional versioned backups (`keep_backups = N`).

### 6.10 Scripting, automation, CLI, and IPC

- **F-10.1 Lua.** Lua 5.4 (via a Rust binding) is the scripting language, matching Hyprland's and Neovim's configuration dialect. `~/.config/omacell/init.lua` runs at startup; `~/.config/omacell/plugins/*/init.lua` are plugins. The API exposes workbooks, sheets, ranges, cells, styles, commands, events (`on_open`, `on_change`, `on_before_save`, `on_recalc`, `on_theme_change`), keymaps, the status line, prompts, and custom-function registration.
- **F-10.2 Sandbox and trust.** Scripts in user config run with full Lua standard library. Scripts embedded in files run sandboxed (no `io`, `os`, `require` of external modules, no network) unless the file is trusted; trust is per file-hash in `~/.local/state/omacell/trust.toml` and is granted through an explicit command, never a modal on open.
- **F-10.3 Macro recorder.** Recording emits Lua that calls the same commands the UI used — readable, editable, and re-runnable.
- **F-10.4 Command bus.** Every user-facing action is a named command with a JSON argument schema (`cell.set`, `range.sort`, `sheet.add`, `format.number`, `view.freeze`, `file.export`). Keymaps, the command palette, Lua, the CLI, IPC, MCP, and models all invoke the same registry; changesets (§8.6) are built from the same commands. `omacell commands --json` lists them with schemas.
- **F-10.5 CLI.** Mirrors the `omarchy` CLI shape:

  ```
  omacell [file...]                          # GUI
  omacell --tui [file]                       # terminal UI
  omacell convert in.xlsx out.csv --sheet Data --range A1:F1000
  omacell query  book.xlsx 'Summary!A1:D20' --json | jq
  omacell set    book.xlsx 'Inputs!B3' 0.07
  omacell eval   book.xlsx '=XIRR(Cash!B2:B40,Cash!A2:A40)'
  omacell recalc book.xlsx --write
  omacell run    script.lua book.xlsx
  omacell fn list --json
  omacell fn doc XLOOKUP
  omacell config check | edit | reset | show <key> --explain
  omacell theme show | reload
  omacell ai setup | card book.xlsx --level columns --json | refresh | freeze | usage | log
  omacell agent "Reconcile Inputs against Ledger" [--diagnose]   # hand-off to the Omarchy default agent
  omacell mcp [--socket]                                          # MCP server for any agent harness
  omacell changeset list | show | apply | revert | export --omc
  omacell audit book.xlsx --json
  omacell setup omarchy                      # installs template + hook into ~/.config
  omacell commands --json
  omacell ipc <command> [json]               # talk to a running instance
  ```

  All read commands take `--json`; all write commands support `--dry-run`. Exit codes are documented. Shell completions ship for bash, zsh, and fish.
- **F-10.6 IPC.** A running instance listens on `$XDG_RUNTIME_DIR/omacell/<pid>.sock` (JSON-lines: request `{id, cmd, args}`, reply `{id, ok, result|error}`, plus event subscriptions). This is what the `theme-set` hook, Hyprland bindings, shell plugins, and agents use. `omacell ipc` targets the focused instance by default.

### 6.11 Printing and export

- **F-11.1** Page setup per sheet: orientation, paper size, margins, scaling or fit-to-pages, print area, print titles, headers/footers with fields, page breaks (manual and preview), gridlines/headings on/off, black-and-white.
- **F-11.2** PDF export from Omacell's own renderer (identical output to the screen, fonts embedded); print via CUPS (`lp`) with a print-preview mode in the main window.
- **F-11.3** Range/sheet export to PNG/SVG (for charts and selections) and to Markdown/HTML tables.

### 6.12 Accessibility and localization (functional)

- **F-12.1** Screen readers receive cell address, value, formula presence, and format description via the toolkit's AT-SPI bridge; every panel is reachable by keyboard; focus is always visible.
- **F-12.2** UI strings are localizable (Fluent); function names are canonical English with optional localized aliases (tier 2); number, date, currency, and list-separator conventions follow `LC_*` with per-workbook overrides.

## 7. Omarchy integration specification

This section is the contract between Omacell and the Omarchy 4.x host. Everything here is re-verified against each Omarchy release channel (stable, RC, edge) in CI (§14).

### 7.1 Theming contract

**Rule: Omacell has no themes. It has color roles, and a mapping from the active Omarchy theme to those roles.**

**Active theme resolution**, first hit wins:

1. `~/.local/state/omarchy/current/theme/` (Omarchy 4.x; `theme.name` alongside gives the name)
2. `~/.config/omarchy/current/theme/` (Omarchy 3.x)
3. None → built-in neutral palette, light/dark chosen from the desktop portal's `color-scheme` setting (portable behavior on non-Omarchy hosts)

**Role source**, first hit wins per role, so partial overrides are possible:

1. `--theme <file>` / `$OMACELL_THEME`
2. `~/.config/omacell/theme.toml` (user override; may define any subset of roles)
3. `<active-theme>/omacell.toml` — rendered by Omarchy from `~/.config/omarchy/themed/omacell.toml.tpl` on every theme switch, once installed by `omacell setup omarchy`
4. Built-in mapping applied directly to `<active-theme>/colors.toml` — the zero-configuration path; identical to (3) by construction, so the template exists for users who want to change the *mapping*, not for correctness

**Role set.** Defined once in Appendix C. Roles cover: surfaces (`background`, `surface`, `header_background`), text (`foreground`, `muted`, `header_foreground`), structure (`grid_line`, `pane_divider`, `frozen_edge`), state (`cursor`, `selection`, `selection_border`, `active_header`, `hover`), semantics (`error`, `warning`, `success`, `info`, `link`), formula reference cycle (eight colors), and the chart palette (eight colors). Every role maps to a `colors.toml` key or a `mix` of keys, so any theme that is valid for Omarchy is valid for Omacell.

**Light mode.** `mode = "light"` in `colors.toml` (or the legacy `light.mode` file) flips mixing directions (grid lines are mixed *darker* on light themes) and selects light-appropriate defaults for data bars and color scales.

**Contrast guarantee.** At load, derived roles (never the theme's own `background`/`foreground`) are nudged along the theme's neutral ramp until text-on-surface pairs reach WCAG AA (4.5:1) and structural lines reach 1.5:1. The nudge is logged at debug level so theme authors can see it. Users can disable it (`[appearance] enforce_contrast = false`).

**Live reload.** Three mechanisms, any of which suffices: (a) an inotify watch on the `current/` directory catches the atomic theme swap; (b) the `theme-set.d` hook installed by `omacell setup omarchy` runs `omacell ipc theme.reload`; (c) `SIGUSR1` triggers a reload for environments without either. Reload re-derives roles, re-renders without losing edit state, and re-tints chart defaults.

**Shell tokens.** When `<active-theme>/shell.toml` exists, Omacell reads its `[font]`, `[spacing]`, and corner-radius values so panels, popups, and the command palette share the shell's typography scale, padding, and rounded/sharp corner setting. `~/.config/omarchy/shell.toml` overrides are respected because Omarchy has already merged them by the time the file is read.

**Cell-level colors from files are never re-themed.** A cell an Excel user made red stays red. Theme roles govern chrome and *defaults* only.

**TUI theming is free.** The terminal front-end paints with ANSI palette indices, which Omarchy already themes in Foot/Alacritty/Ghostty/Kitty. 24-bit colors are used only for values that came from a file.

### 7.2 Fonts and text size

- UI chrome (headers, formula bar, status line, panels) uses the fontconfig `monospace` alias, which Omarchy points at the font chosen in *Style > Font*. The `font-set.d` hook triggers a refresh; fontconfig changes are also watched.
- UI text size follows Omarchy's display text size knob by reading the shell `[font]` scale, falling back to GTK's `text-scaling-factor` and then to 11 pt. Users override with `[appearance] ui_font_size`.
- Cell text defaults to the same monospace font (`cell_font = "monospace"`), which suits numeric grids and Omarchy's aesthetic. `cell_font = "sans-serif"` restores the proportional default Excel users expect. Per-cell fonts from files are honored with a substitution table (Calibri/Aptos → Carlito, Arial → Liberation Sans, Times New Roman → Liberation Serif, Cambria → Caladea) so layouts hold; missing fonts are listed in the file-info panel.
- Nerd Font glyphs are used for status-line indicators when available and fall back to ASCII.

### 7.3 Hyprland and tiling behavior

- Native Wayland client with the `xdg-decoration` protocol declining server-side decorations by default (Omarchy windows are borderless-tiled; the app draws no title bar unless floating).
- App id/class `omacell`; window title `<file> — Omacell` with a `•` prefix when dirty, so the bar and window switcher show state.
- No minimum window size beyond one cell plus chrome. At widths below a configurable threshold the formula bar collapses into the status line; panels become overlays.
- Dialog-free by design: format, validation, conditional-format, find/replace, comments, pivot builder, chart builder, and page setup are all *panels* docked left/right/bottom (position configurable). The only floating surfaces are transient popups (autocomplete, tooltips, dropdowns), which set the correct `xdg_popup` role so Hyprland places them.
- Multiple windows per instance are supported (one workbook per window, or views of one workbook). Session state — open files, window→workspace mapping, panel layout, selection — is saved on exit and restored on launch when `[session] restore = true`.
- Fractional scaling is honored per output via `wp-fractional-scale-v1`; crisp lines at 1.25×/1.5×/1.75× are a test case.
- Documented Hyprland snippets ship with the package and are printed by `omacell setup omarchy --show-hyprland`:

  ```lua
  -- ~/.config/hypr/bindings.lua  (pick any chord that is free on your machine)
  o.bind("SUPER + ALT + X", "Spreadsheet", "omacell")
  ```

### 7.4 Keybinding conventions and conflict avoidance

- Omacell never binds `Super` chords; they belong to Hyprland/Omarchy.
- Default bindings use `Ctrl`, `Alt`, `Shift`, and function keys, matching Excel; the modal keymap uses unmodified keys in Normal mode.
- `omacell keys check` reads `~/.config/hypr/bindings.lua` and the Omarchy defaults (via `omarchy` if available), lists any chord the app also binds, and suggests remaps. This runs during `omacell setup omarchy`.
- `F1` opens the in-app key overlay (searchable, grouped by mode), the app-level counterpart of Omarchy's hotkey menu.

### 7.5 Configuration layering and lifecycle

```
 highest precedence
 ┌───────────────────────────────────────────────────────────────┐
 │ CLI flags            --set appearance.grid_lines=false         │
 │ Environment          OMACELL_*                                  │
 │ Workbook settings    stored in the file (calc mode, date system)│
 │ ~/.config/omacell/   config.toml · keys.toml · theme.toml ·     │  ← yours; never
 │                      init.lua · plugins/                        │    touched by updates
 │ Active Omarchy theme omacell.toml or derived from colors.toml   │  ← changes when the
 │                      + shell.toml tokens                        │    theme changes
 │ /usr/share/omacell/default/                                     │  ← package-owned;
 │                      config.toml · keys/*.toml · themed/*.tpl   │    reset target
 └───────────────────────────────────────────────────────────────┘
 lowest precedence
```

- Package defaults are complete and commented; the user files start empty (or absent). A user file only needs the keys it changes.
- `omacell config reset [file]` moves the current user file to `~/.local/state/omacell/backups/<timestamp>/` and restores defaults — the `omarchy reinstall configs` pattern.
- `omacell config edit [file]` opens the file with `$EDITOR` (Neovim on Omarchy) and validates on save, printing errors with line numbers.
- `omacell config show <key> --explain` prints the effective value and which layer set it.
- All user files are watched; valid edits apply live (`[config] live_reload = true`, as Alacritty does). Invalid edits are reported in the status line and ignored, never crash.
- Schema is versioned (`schema = 1`); migrations rewrite user files only with a backup and a status message, mirroring `omarchy update` migrations.

### 7.6 Shell and desktop integration

- `.desktop` entry with MIME associations for `.xlsx`, `.xlsm` (opened with macros inert), read-only `.xls`, `.csv`, `.tsv`, `.ods`, `.omc`; `xdg-open file.xlsx` launches Omacell when the user sets it as default (the installer prints the `xdg-mime` line; it never changes defaults silently).
- Omarchy menu entry: `omacell setup omarchy` offers to add `~/.config/omarchy/extensions/omarchy-menu.jsonc` rows (e.g. a *Spreadsheet* row and a *New from clipboard* row that pastes the clipboard table into a fresh workbook). The user confirms before the file is written.
- Notifications through `omarchy-notification-send` when present (Omarchy's own convention; it renders through the shell), otherwise the standard freedesktop D-Bus interface: long recalc finished, autosave recovered, export complete, changeset proposed by an external agent. Off by default except recovery and agent proposals.
- Clipboard through `wl-clipboard`-compatible protocols; Omarchy's unified clipboard history sees the `text/plain` representation.
- Optional delight: with Omarchy's OCR text extraction, `omacell paste --from-ocr` parses the captured text as a delimited table and drops it at the cursor.
- Icon shipped in `hicolor` at all standard sizes plus a symbolic variant that follows the icon theme named in the active theme's `icons.theme`.

### 7.7 Terminal front-end

- `omacell --tui` renders the grid in any terminal with 256-color or true-color support; sixel/kitty-graphics chart previews are optional.
- Same commands, same keymaps (classic or modal), same files. Mouse support in terminals that report it.
- Designed for tmux/SSH: no dependence on a display; the IPC socket still works, so a GUI instance and a TUI instance can drive the same workbook with cooperative locking.

### 7.8 Packaging and updates

- Arch package published to the AUR as `omacell` (source) and `omacell-bin` (prebuilt), installable with `omarchy-pkg-add omacell-bin`. PKGBUILD lives in the repository.
- Installs to `/usr/bin/omacell`, `/usr/share/omacell/{default,themed,hooks,docs}`, `/usr/share/applications`, `/usr/share/icons/hicolor`, `/usr/share/man/man1`, and shell completions. **Nothing is written under `/usr/share/omarchy` or `~/.config/omarchy` by the package**; only `omacell setup omarchy`, run by the user, writes the template and hook into `~/.config/omarchy/`.
- Post-install message tells the user to run `omacell setup omarchy`; the GUI also offers it once on first launch with a status-line notice, never a modal.
- Releases follow semver; a `stable`/`edge` split is unnecessary at the package level because Omarchy's update channels already govern the host.

### 7.9 Agent friendliness

Omarchy's own CLI exists partly so AI agents can operate the system. Omacell follows suit: `omacell commands --json` publishes every command with its argument schema; every query has `--json`; every mutation has `--dry-run`; `omacell query` supports `--format json|csv|md`; error messages are machine-parseable (`{code, message, hint}`); and the IPC event stream (`cell.changed`, `recalc.done`, `file.saved`, `changeset.proposed`) lets an agent react without polling. The AI-specific surfaces built on this — the shipped skill, the MCP server, `omacell agent` hand-off, and changesets — are specified in §8.5 and §8.6.

## 8. AI-native design

Omarchy treats AI coding agents as first-class citizens without picking a favorite, keeps agentic features off until the user chooses an agent, and ships skills that teach agents how to operate the system. Omacell applies the same stance to the spreadsheet. "AI-native" here means five specific things:

1. **The workbook is legible to models** — a compact, redactable context object rather than screenshots or raw dumps.
2. **Models act through the command bus, never through the UI** — so every AI action is a reviewable, undoable changeset.
3. **AI is available where spreadsheet work happens** — in cells, in the formula bar, in the palette, on import, in an audit, and in a docked agent panel.
4. **External agents are first-class operators** — the user's default Omarchy agent or any MCP client, via a shipped skill, the CLI, and an MCP server.
5. **Nothing runs, and nothing leaves the machine, until the user configures it** — and local models are the default path.

Requirement identifiers in this section are `A-x.y`.

### 8.1 Posture and defaults

- **A-1.1** `[ai] enabled = false` ships as the default. With AI disabled the application is complete: no feature degrades, no UI nags, and the only trace is a status-line hint (`AI: off · omacell ai setup`).
- **A-1.2** `omacell ai setup` detects local servers (Ollama on `localhost:11434` and LM Studio on `localhost:1234` — the two paths Omarchy recommends — plus llama.cpp and vLLM endpoints), detects the Omarchy default agent, and writes only `~/.config/omacell/config.toml`. It never stores secrets in plaintext: cloud keys are referenced by environment variable or by a command (`secret_cmd = "op read op://…"`, `"pass show …"`).
- **A-1.3** Two independent paths; either may be configured alone:
  - **Provider path** — an API endpoint used by in-app features (functions, palette, assist, panel). OpenAI-compatible HTTP and the Anthropic Messages API cover local and cloud alike (ADR-005). OpenRouter is a provider like any other.
  - **Agent path** — the user's default Omarchy agent (`claude`, `codex`, `opencode`, `pi`, … whatever `omarchy default agent` names) operating Omacell from the outside through the skill, the CLI, and the MCP server. No key and no provider configuration: the agent brings its own.
- **A-1.4** Nothing is sent on file open, on recalculation of non-AI cells, or in the background. Every request traces to a user action, to recalculation of an AI cell whose inputs changed, or to an agent command.
- **A-1.5** Loopback endpoints are local: the privacy level defaults to `full` for them and to `schema` for anything else (§8.7).

### 8.2 The workbook card (what models see)

- **A-2.1** Omacell maintains a *workbook card*: a JSON description of the workbook sized to a token budget. Levels: `summary` (sheets, dimensions, defined names, tables, formula counts, most-used functions, external references, validation and conditional-format counts), `columns` (adds per-column name, inferred type, null share, distinct count, min/max, k sample values, header-row guess), `sample` (adds N representative rows per table or region), `full` (a requested range's values, paginated).
- **A-2.2** Cards are built from the model, not from the screen; the TUI and CLI produce identical cards (`omacell ai card book.xlsx --level columns --json`).
- **A-2.3** Selection-aware focus: a request that originates from a selection centers the card on that region plus its precedents and dependents (from the dependency graph) before spending the remaining budget on the rest of the workbook.
- **A-2.4** Redaction (§8.7) is applied before a card leaves the process; redacted cells appear as typed placeholders (`[REDACTED:email]`) so structure survives.

### 8.3 AI in cells (functions)

- **A-3.1 Surface.** `AI(prompt, [context], [options])`, `AI.EXTRACT(text, what)`, `AI.CLASSIFY(text, categories)`, `AI.FILL(examples, inputs)` (by-example transformation — Flash Fill with a model; spills), `AI.TABLE(prompt, [columns])` (spills a table), `AI.TRANSLATE(text, language)`. `options` is a small inline object: `"type":"number|text|boolean|json"`, `"model":"fast|default|strong"`, `"cache":"on|off"`. Precedents: Excel's `COPILOT()` and Google Sheets' `AI()`.
- **A-3.2 Semantics.** AI functions are *non-volatile*. Results are memoized by a hash of (task, prompt-template version, model, inputs). Changing an input re-queries; recalculation alone does not. `ai.refresh` (cell, range, or workbook) forces a re-query; `Ctrl+Alt+F9` does not unless `[ai.functions] refresh_on_full_recalc = true`.
- **A-3.3 Asynchrony.** AI cells are asynchronous graph nodes. A recalculation pass evaluates everything else immediately, marks AI cells and their dependents *stale* (hatched, §10.2), issues batched requests, and runs a second wave when results land. `omacell recalc --wait` blocks until the graph settles. Given the cache, results are deterministic.
- **A-3.4 Batching and limits.** Pending cells sharing a task are batched into one request carrying a JSON array (default 50 rows). Guardrails: `max_cells_per_recalc`, `max_requests_per_minute`, `max_tokens_per_request`, and a per-workbook budget. Crossing a threshold turns the recalculation into a status-line confirmation with an estimate — never a silent spend.
- **A-3.5 Provenance.** Each cached result records model, provider, timestamp, prompt hash, and token counts; hover or `Ctrl+Shift+E` shows it; `ai.pin` freezes a cell's result; `ai.freeze` converts a range to plain values.
- **A-3.6 Failure.** No provider, provider down, or over budget yields `#N/A` with a machine-readable hint; an existing cached value is kept and shown stale (`[ai.functions] keep_stale = true`).
- **A-3.7 Files.** Results and provenance are stored in the workbook (the `.xlsx` custom part; `aicache` records in `.omc`), so a workbook opens and displays correctly on a machine with no AI configured. In `.xlsx`, AI formulas are written with their cached values: Excel displays those values and returns `#NAME?` only if it recalculates the cell. Saving warns once per workbook; `ai.freeze` or `[ai.functions] xlsx_export = "values"` produces Excel-safe files. Excel's `COPILOT()` is imported as an inert formula with its cached value (mapping it to `AI()` is an open decision, §16.2).
- **A-3.8 Typing.** Results are typed by the `type` option, with number parsing under the workbook locale; untyped results are text. AI functions never return formulas; *insert as formula* is a separate, user-invoked action.

### 8.4 AI in the formula bar, the palette, and the import preview

- **A-4.1 Natural-language commands.** `Ctrl+Shift+A` (or a leading `?` in the command palette; `<leader>a` / `:ai` in modal) takes a sentence and returns a *plan*: the exact command-bus commands with arguments and affected ranges — for example `range.sort Data!A1:F400 by F desc` followed by `cf.add Data!F2:F400 cell_value > 10000 → style:error`. The plan is shown before anything runs; `Enter` applies it as one changeset (§8.6). The model sees only the command registry schema (`omacell commands --json`) and the workbook card, which keeps it grounded and makes the feature testable offline.
- **A-4.2 Formula assistant.** Commands on the current cell or selection: `ai.formula.generate` (from a description; uses headers, samples, names, and tables), `ai.formula.explain` (plain language plus a step trace aligned with Evaluate Formula), `ai.formula.fix` (fed the evaluator's error diagnosis), `ai.formula.refactor` (`VLOOKUP`→`XLOOKUP`, nested `IF`→`IFS`/`SWITCH`, introduce `LET`, absolute-reference repair). Generated formulas are parsed and evaluated in a scratch context before they are proposed; references the model chose are highlighted for verification.
- **A-4.3 Inline completion.** Ghost-text completion in the formula bar and the in-cell editor, accepted with `Tab`, debounced, cancellable, using the `fast` model slot. `[ai.completion] mode = "auto"` means on only when the fast model is local; `on` and `off` override.
- **A-4.4 Import assistant.** After sniffing (§6.9), `ai.import.assist` proposes column names, types, unit extraction from headers ("Pressure (psi)"), date-format resolution, whitespace and category normalization — as a reviewable proposal inside the import preview. Nothing is applied without acceptance; the Excel auto-conversion lesson still governs.
- **A-4.5 Describe and audit.** `ai.describe` summarizes a sheet or range. `ai.audit` pairs the auditing engine (§6.3) with a model: hard-coded constants inside formulas, inconsistent formulas within a column, ranges that stop a row short of the data, circular references, and unit mismatches implied by headers. Findings land in a panel with jump links and, where safe, a one-key fix that is itself a changeset. `omacell audit --json` runs the deterministic part headless.

### 8.5 Agents: skill, MCP server, and hand-off

- **A-5.1 Skill.** Omacell ships an agent skill (`SKILL.md`, in the same format Omarchy's own skill uses) that teaches a coding agent to inspect, query, and modify workbooks through the CLI and the MCP tools; to prefer changeset proposals over direct writes; and to run `omacell recalc --wait` and `omacell audit --json` before declaring a task done. `omacell setup omarchy` links it into the same locations Omarchy uses for its skill (`~/.agents/skills/`, `~/.claude/skills/`, `~/.codex/skills/`, `~/.pi/agent/skills/`, `~/.gemini/config/skills/`), so the user's default agent picks it up without configuration.
- **A-5.2 MCP server.** `omacell mcp` (stdio) and `omacell mcp --socket` expose tools: `workbook_open/list/save`, `sheet_list/add/rename`, `range_read` (values, formulas, formats; paginated), `range_write`, `formula_set`, `command_run` (any registry command), `commands_list`, `recalc`, `audit`, `card`, `changeset_propose/apply/revert/list`, `export`, and `render` (PNG of a range for vision-capable models; GUI only). Resources: `omacell://<file>/card`, `omacell://<file>/<sheet>`. Registration is one line per harness (for example `claude mcp add omacell -- omacell mcp`).
- **A-5.3 Hand-off.** `omacell agent "<prompt>"` and the palette command *Hand to agent* run `omarchy agent prompt` with the workbook path, the current selection, and the skill in reach, from the workbook's directory. Because Omarchy launches agents in their unattended modes, the skill instructs the agent to submit changesets rather than write files directly; a running Omacell instance surfaces them for review (§8.6), and a headless run leaves them in `omacell changeset list`. When no default agent is chosen, these entries are hidden — the same behavior Omarchy applies to its own agentic features.
- **A-5.4 Diagnosis triggers.** Analogous to Omarchy's crash diagnosis: a `#REF!` cascade after a structural edit, a new circular reference, or a failed import offers *Diagnose with agent* in the status line, which runs `omacell agent diagnose` with a diagnostic bundle (error explanations, the dependency neighborhood, recent undo history). Hidden when no agent is chosen; silenced with `[ai.agent] diagnose_offers = false`.
- **A-5.5 Usage.** Token usage per provider is recorded locally (`omacell ai usage --json`). If the Omarchy agents panel's record format is documented and stable, Omacell writes compatible records so its usage appears there; otherwise a small shell plugin provides the same view.

### 8.6 Changesets: the only way a model touches the grid

- **A-6.1** A *changeset* is a first-class object: an ordered list of command-bus commands with their computed inverses, a summary (cells, rows, columns, sheets, and styles affected), an origin (user, script, palette plan, in-app agent, external agent), and a status (proposed, applied, reverted).
- **A-6.2** Review overlay: proposed changes are rendered in place — before/after values, inserted or deleted structure, style changes — tinted from theme roles (`success`, `error`, `warning`); the user accepts or rejects per item or in bulk from the keyboard. Applying is one undo unit.
- **A-6.3** Review is the default for every AI origin. *Autopilot* is opt-in per session, scoped to sheets or ranges, capped by operation count, always undoable, and structurally unable to change trust, network, scripting, or AI settings, run scripts, follow external links, or write any file other than the open workbook.
- **A-6.4** Changesets are exportable (`omacell changeset export --omc`) and applicable from files, so an agent working headless can propose to a GUI session, and so a proposal can be reviewed in an ordinary diff tool.

### 8.7 Privacy, safety, and trust

- **A-7.1 Data levels.** `[ai.privacy] send = "schema" | "sample" | "full"`, global with a per-workbook override stored in the workbook, so a file carries its own policy. `schema` sends structure and formulas only; `sample` adds bounded samples; `full` sends requested ranges. Loopback providers default to `full`, others to `schema`.
- **A-7.2 Redaction.** Ranges and columns can be marked `ai.redact`. Pattern detectors (email, phone, card-like numbers, national-ID shapes, IBAN) suggest redactions at first send and remain suggestions until accepted. Redaction applies to cards, cell inputs, and `render` output alike.
- **A-7.3 Visibility.** A status-line segment shows the active provider (local or cloud), the privacy level, and a session counter of requests and bytes sent; hover shows the last request's summary. `omacell ai log` reads the audit log (`~/.local/state/omacell/ai/log.jsonl`: task, provider, model, sizes, hashes, latency; content only when `log_content = true`).
- **A-7.4 Cell content is data, not instructions.** Prompts are templates; workbook content is fenced and labeled as data in every request; no tool available to a model can alter trust, network, scripting, or AI policy; mutations go through changesets; `AI()` results are values. A standing test suite (§14) pushes instruction-shaped cell contents through every AI feature and asserts no behavioral change.
- **A-7.5 No training, no upload.** Omacell never uploads files to any service beyond the request payloads described here, and the manual says so. Provider terms are the user's to read; `omacell ai setup` prints the endpoint it will use and stops.

### 8.8 Tunability of the AI layer

Consistent with §9, the AI layer is text all the way down:

- **Model routing** — `[ai.models]` maps task slots (`fast`, `default`, `strong`, `agent`, `vision`) to `provider:model` strings; any task can be pinned to a slot.
- **Prompts** — every task's system and user templates are Markdown files in `/usr/share/omacell/default/ai/prompts/`, overridable file by file in `~/.config/omacell/ai/prompts/`, and versioned: a template change invalidates that task's cache.
- **Skills** — the in-app agent loads `SKILL.md` directories from `~/.config/omacell/ai/skills/` in the same format as Omarchy's and the coding agents' skills (ADR-006), so one skill can serve the in-app agent and an external one.
- **Lua** — `omacell.ai.task(name, {prompt=…, tools=…, schema=…})` registers new tasks, `omacell.ai.fn("MY.NAME", …)` registers AI-backed functions, and `on_ai_request`/`on_ai_response` hooks allow local pre- and post-processing (or routing through a corporate gateway) without patching the application.
- **Providers** — `[ai.providers.<name>]` blocks: `kind = "openai_compatible" | "anthropic"`, `endpoint`, `secret_env` or `secret_cmd`, `timeout`, `headers`, `local = true|false`. Adding a provider is adding a block.

## 9. Tunability model

"Highly tunable" is specified as: **every property in the table below has a key, a default, a layer, a scope, and a reload behavior.** Appendix B is a complete, commented `config.toml`.

### 9.1 What is tunable, and where

| Area | File / section | Representative keys | Scope | Hot reload |
|---|---|---|---|---|
| Appearance | `config.toml [appearance]` | `cell_font`, `cell_font_size`, `ui_font_size` (`"system"` follows Omarchy), `grid_lines`, `grid_line_style`, `row_height`, `column_width`, `cell_padding`, `cursor_style` (`block|underline|outline`), `selection_style`, `show_formula_bar`, `show_status_line`, `show_sheet_tabs`, `sheet_tabs_position`, `corner_style` (`"system"`), `zebra_rows`, `enforce_contrast`, `animation` (`"system"|on|off`) | global, per-window overrides | yes |
| Theme mapping | `theme.toml` / `~/.config/omarchy/themed/omacell.toml.tpl` | every role in Appendix C | global | yes |
| Behavior | `config.toml [behavior]` | `enter_moves` (`down|right|none`), `autocomplete`, `autocorrect`, `formula_hints`, `reference_style` (`A1|R1C1`), `default_sheets`, `date_system`, `precision_as_displayed`, `smart_paste`, `fill_prompt` | global; some per workbook | yes |
| Calculation | `config.toml [calc]` | `mode` (`automatic|manual`), `threads` (`"auto"`), `iterative`, `max_iterations`, `max_change`, `volatile_on_open` | global default; stored per workbook | yes |
| Keys | `keys.toml` | `model`, `leader`, `[bindings.normal]`, `[bindings.insert]`, `[bindings.visual]`, `[bindings.command]`, per-key `= "command.name"` or `= { cmd = ..., args = {...} }` | global | yes |
| Locale | `config.toml [locale]` | `language`, `decimal_separator`, `thousands_separator`, `list_separator`, `date_format`, `currency`, `first_weekday`, `localized_function_names` | global; per workbook for separators | yes |
| Files | `config.toml [files]` | `default_format` (`xlsx|omc`), `autosave_interval`, `keep_backups`, `follow_external_links`, `csv.*` defaults (delimiter, encoding, type inference), `xlsx.preserve_unknown_parts` | global | yes |
| Session | `config.toml [session]` | `restore`, `recent_files`, `workspace_binding` | global | on next launch |
| Panels & layout | `config.toml [layout]` | `panel_side`, `panel_width`, `formula_bar_lines`, `compact_below_width`, `status_line = ["mode","cell","stats","calc","theme"]` | global, per-window | yes |
| Integrations | `config.toml [integrations]` | `omarchy` (`auto|on|off`), `notifications`, `menu_entries`, `ocr_paste`; deprecated `libreoffice_fallback` is accepted but ignored | global | on next launch |
| Network | `config.toml [network]` | `enabled` (default `false`), `allow_functions = []`, `proxy` | global; per workbook allowlist | yes |
| Scripting | `init.lua`, `plugins/`, `config.toml [scripting]` | `enabled`, `trusted_dirs`, `embedded_scripts` (`sandbox|ask|deny`) | global; per-file trust | plugins on next launch; `init.lua` via `:source` |
| AI | `config.toml [ai]`, `[ai.providers.*]`, `[ai.models]`, `[ai.privacy]`, `[ai.functions]`, `[ai.completion]`, `[ai.agent]` | `enabled`, provider blocks, slot routing (`fast|default|strong|agent|vision`), `send` level, redaction, budgets, `completion.mode`, `review`, `autopilot_scope`, `diagnose_offers` | global; privacy level and budgets per workbook | yes (provider changes apply to the next request) |
| AI prompts & skills | `~/.config/omacell/ai/prompts/*.md`, `~/.config/omacell/ai/skills/*/SKILL.md` | per-task system and user templates; skill directories | global | yes (a template edit invalidates that task's cache) |
| Charts | `config.toml [charts]` | `palette = "theme"`, `default_type`, `line_width`, `font` | global; per chart | yes |
| TUI | `config.toml [tui]` | `unicode_borders`, `truecolor`, `mouse`, `graphics` (`auto|sixel|kitty|off`) | global | yes |

### 9.2 Tunability rules

1. **No hidden state.** If the UI can change it, a key exists and `omacell config show --all --json` reports it.
2. **Every default is visible.** `/usr/share/omacell/default/config.toml` is the documentation.
3. **Overrides are sparse.** User files contain only what differs. `omacell config diff` shows the delta.
4. **Provenance is queryable.** `--explain` names the layer.
5. **Reload is safe.** A bad edit never crashes and never loses work; the last good configuration stays active.
6. **Escape hatch.** Anything not expressible in TOML is expressible in `init.lua`, which runs after configuration is loaded and can register commands, keymaps, functions, and event handlers.
7. **Theme is data, not code.** The color mapping is a template of `{{ placeholders }}`, so a theme author or user can change it without touching Lua.

## 10. User experience specification

### 10.1 Layout

```
┌──────────────────────────────────────────────────────────────────────┐
│ Sheet1 · Sheet2 · Data +                                    (tabs)  │  optional, top or bottom
├──────────────────────────────────────────────────────────────────────┤
│ B7   ▸ =XLOOKUP($A7,Rates[Code],Rates[Rate])           (formula bar) │  expandable to N lines
├────┬─────────┬─────────┬─────────┬─────────┬─────────┬──────────────┤
│    │    A    │    B    │    C    │    D    │    E    │              │
│  1 │ Code    │ Rate    │ Amount  │ Total   │         │   panel      │
│  2 │ …       │         │         │         │         │   (docked,   │
│  … │         │  grid   │         │         │         │   optional)  │
├────┴─────────┴─────────┴─────────┴─────────┴─────────┴──────────────┤
│ NORMAL  B7  Sum 1,204.50  Avg 301.13  Cnt 4   Auto ⟳   tokyo-night  │  status line
└──────────────────────────────────────────────────────────────────────┘
```

- Default chrome is sheet tabs, formula bar, grid, status line. No toolbar, no menu bar, no ribbon. An optional classic menu bar (`[layout] menu_bar = true`) exists for people who want discoverability from `Alt`.
- The **command palette** (`Ctrl+Shift+P`, `:` in modal, `F1` for keys) is the primary discovery surface: fuzzy search over all commands with their current key, recent commands first, argument prompts inline. A leading `?` switches it to natural language and returns a plan (§8.4).
- The **status line** is a configurable list of segments (mode, address, selection stats, calc status with a spinner during long recalc, circular-reference warning, theme name, file dirty flag, zoom, and — when AI is enabled — provider, privacy level, and session sends). Segments are clickable.
- **Panels** replace dialogs. One panel is visible at a time by default; `Esc` closes it and returns focus to the grid. Panels are also reachable as commands, so a Lua script can open the pivot builder pre-filled. The agent panel and the changeset review panel (§8.5, §8.6) are panels like any other.

### 10.2 Interaction rules

- Keystroke to paint under 16 ms at any zoom; scrolling and selection never block on recalc, which runs off the UI thread with a status indicator and per-cell "stale" hatching for values not yet updated.
- Errors are shown in the cell as Excel's error value, with a hover/`Ctrl+Shift+E` explanation and a one-key jump to the offending precedent.
- Destructive actions (delete sheet, remove duplicates, replace all) show a status-line confirmation with count, not a modal; `u`/`Ctrl+Z` reverses every one of them.
- Long operations (import, recalc, export) run with a cancellable progress segment in the status line; the grid stays usable.
- Zoom (`Ctrl+scroll`, `Ctrl+Alt+=` / `Ctrl+Alt+-`, `Ctrl+Alt+0`) scales the grid only; chrome follows the system text size.
- Focus is always visible: a themed cursor outline in the grid, an accent border on the focused panel field.
- AI never writes directly. Proposals appear as a changeset overlay (§8.6) with per-item accept/reject; the grid stays editable underneath, and a pending proposal never blocks saving.

### 10.3 First run

No wizard. The first GUI launch shows an empty workbook with a two-line status message: the theme in use and the suggestion to run `omacell setup omarchy`. `F1` shows keys. Nothing else interrupts.

## 11. Architecture

### 11.1 Overview

```
┌────────────────────────────────────────────────────────────────────────┐
│                              Front-ends                                │
│  ┌────────────────┐   ┌────────────────┐   ┌────────────────────────┐  │
│  │ GUI (Wayland)  │   │ TUI (ratatui)  │   │ CLI · IPC · MCP (JSON) │  │
│  └───────┬────────┘   └───────┬────────┘   └───────────┬────────────┘  │
│          └────────────────────┴────────────────────────┘               │
│                             │  Command bus                             │
│                             │  name + JSON args · changesets · events  │
├─────────────────────────────┼──────────────────────────────────────────┤
│                        omacell-core                                    │
│   workbook model · parser · evaluator · dependency graph · styles ·    │
│   number formats · tables · pivots · charts model · undo/redo          │
├────────────┬────────────┬────────────┬─────────────┬───────────────────┤
│ omacell-fn │ omacell-io │ omacell-lua│ omacell-ai  │ omacell-conf      │
│ function   │ xlsx ods   │ sandboxed  │ providers · │ layered TOML ·    │
│ library +  │ csv json   │ Lua API ·  │ card · tasks│ theme resolution ·│
│ registry   │ parquet omc│ plugins ·  │ cache ·     │ watchers ·        │
│            │ pdf svg    │ recorder   │ redaction   │ Omarchy adapters  │
└────────────┴────────────┴────────────┴─────────────┴───────────────────┘
   Omarchy host: theme dir · shell.toml · hooks · fontconfig · portals ·
                 default agent · skill directories · local LLM servers
```

Language: Rust for everything below the front-end line (memory safety on untrusted file input, performance, one binary). The GUI toolkit is an open decision (ADR-001).

### 11.2 Architecture decision records (open unless marked)

**ADR-001 — GUI toolkit.** Options:

| Option | For | Against |
|---|---|---|
| **Qt Quick (QML) via `cxx-qt`, custom scene-graph grid item** — *recommended for the spike* | Omarchy 4's shell is Qt Quick; Omacell can consume the same `shell.toml` tokens and look like part of the shell. Mature Wayland fractional scaling, IME, AT-SPI accessibility, text shaping. QML is runtime-tunable, which makes theme hot-reload trivial. | Two-language boundary; Qt build/link weight; must still hand-write the grid renderer for performance. |
| GTK4 (`gtk4-rs`), custom `GtkDrawingArea`/GSK grid | Many Omarchy GUI apps are GTK; good a11y; single-language via bindings. | GTK4 widget layer is heavy for a grid; theming via CSS is less direct than QML properties. |
| Pure Rust (`iced`/`egui` + `wgpu`) | One language, one binary, full control, fastest iteration. | Accessibility and IME are the weak points; text shaping quality must be proven; less "native" feel. |

Decision criteria for the M0 spike: 1M-row scroll at 60 fps on integrated graphics, theme hot-reload without flicker, correct fractional scaling at 1.25×/1.5×, CJK IME input into a cell, and Orca reading the active cell.

**ADR-002 — Engine: build or adopt.** Evaluate IronCalc (Rust, open source, Excel-compatible formula model, `.xlsx` I/O) as the base for `omacell-core`/`omacell-fn`. Criteria: license compatibility, dynamic-array and `LAMBDA` coverage, range-aware dependency graph performance at 1M formulas, willingness to upstream. If adopted, Omacell contributes rather than forks. If not, the data structures in §11.3 are the build plan.

**ADR-003 — Native format is `.xlsx` (decided, §6.9).** `.omc` is a sibling for text workflows, not a replacement.

**ADR-004 — Scripting language is Lua (decided, §6.10).** Python is available to plugins through a subprocess bridge (`omacell run --python`) rather than embedded, to keep the binary small and the sandbox simple.

**ADR-005 — AI providers are wire protocols, not SDKs (decided, §8.1).** Omacell speaks two protocols — OpenAI-compatible chat completions (which covers Ollama, LM Studio, llama.cpp, vLLM, OpenRouter, and most cloud vendors) and the Anthropic Messages API — with structured output and tool calling on both. No per-vendor SDKs; a provider is a TOML block. Rationale: vendor neutrality (Omarchy's stance), a small binary, and local-first by construction.

**ADR-006 — One skill format (decided, §8.8).** In-app agent skills use the same `SKILL.md` layout as Omarchy's shipped skill and the coding agents' skill directories, so a skill written for the in-app agent also works when the workbook is handed to Claude Code, Codex, Pi, or OpenCode.

### 11.3 Core data structures

- **Cell storage.** Per sheet, cells live in 256×256 blocks keyed by block coordinate in a hash map; within a block, a dense array of compact cell slots plus a bitmap. This gives O(1) addressing, cheap whole-row/column scans, and small memory for sparse sheets. Populated-cell budget: ≤ 64 bytes amortized for a numeric cell without style.
- **Values.** A 16-byte tagged union: number, boolean, error code, interned string handle, array handle. Strings are interned per workbook (shared-string-table shaped, so `.xlsx` export is a table dump).
- **Styles.** Interned style records with reference counts; a cell holds a `u32` style id. Number formats are parsed once into a formatter object and cached.
- **Formulas.** Parsed to a compact AST stored per cell, with references normalized to relative/absolute tokens so fill/copy is a token rewrite, not a re-parse. Shared formulas from `.xlsx` are materialized lazily.
- **Dependency graph.** Nodes are formula cells; edges come from single references and from *range buckets* — an interval index per sheet that maps a range dependency to the row/column blocks it covers — so `SUM(A:A)` does not create a million edges. Dirty propagation walks reverse edges; evaluation orders by DFS with cycle detection; a persisted calc chain (as `.xlsx` does) speeds warm loads.
- **Recalculation.** Work-stealing evaluation of independent subgraphs (rayon); volatile set re-dirtied each pass; deterministic ordering enforced by evaluating within a topological generation before advancing.
- **Undo.** Commands record inverse deltas (cell before/after, style before/after, structural ops as inverse structural ops). Transactions group UI actions. Memory-budgeted with oldest-first eviction.
- **Row/column geometry.** Fenwick trees over heights/widths give O(log n) pixel↔index mapping for scrolling and hit-testing with hidden and custom-sized rows.
- **AI nodes.** AI-function cells are asynchronous graph nodes: evaluation returns the cached value (or stale/`#N/A`) immediately and enqueues a request keyed by content hash; a completion event re-dirties the node and its dependents for a second wave. The cache is a content-addressed store held in the workbook part and mirrored in `~/.cache/omacell/ai/`.
- **Changesets.** Forward commands, inverse commands, a summary computed from affected ranges, an origin, and a status; built on the undo machinery so *apply* is a transaction and *revert* its exact inverse.

### 11.4 Rendering

- Virtualized: only the visible window of cells is laid out; text shaping results are cached by `(string id, style id, zoom)`; a scroll of one row re-shapes one row.
- Layers: fills → gridlines → borders (drawn once per edge, resolved by Excel's border-precedence rules) → text → conditional-format overlays (bars, icons) → selection and cursor → spill/reference outlines.
- Fractional scaling: geometry is computed in logical pixels and snapped to device pixels so gridlines stay one physical pixel.
- Charts render through the same 2-D vector layer used for PDF/SVG export, so screen and export are identical.

### 11.5 Threading

UI thread (input, layout, paint) · calc pool (recalc) · I/O pool (load/save/import in chunks) · AI pool (provider requests, streaming, batching — separate from calc so a slow model never stalls recalculation) · watcher thread (config, theme, fontconfig, lock files) · IPC/MCP thread. All cross-thread communication is message-based; the model is single-writer, with snapshot reads for rendering during long recalcs.

### 11.6 Repository layout

```
omacell/
├─ crates/
│  ├─ core/     model, parser, eval, graph, styles, formats, undo
│  ├─ fn/       function library (generated tables + tests)
│  ├─ io/       xlsx, ods, csv, json, parquet, omc, pdf, svg
│  ├─ lua/      API surface, sandbox, recorder
│  ├─ ai/       providers, workbook card, tasks, cache, redaction, audit log, in-app agent loop
│  ├─ conf/     schema, layering, theme resolution, Omarchy adapters
│  ├─ bus/      command registry, changesets, IPC server/client, MCP server, events
│  ├─ tui/      ratatui front-end
│  ├─ gui/      toolkit front-end (per ADR-001)
│  └─ cli/      binary entry point
├─ default/     shipped defaults: config.toml, keys/classic.toml, keys/modal.toml, themed/omacell.toml.tpl, hooks/theme-set,
│               ai/prompts/*.md, agents/skills/omacell/SKILL.md
├─ packaging/   PKGBUILD, .desktop, icons, completions, man
├─ tests/       corpora: xlsx round-trip, function conformance, csv, themes, AI evals (recorded responses), injection suite
└─ docs/        user manual (mdBook), config reference generated from schema
```

## 12. Non-functional requirements

### 12.1 Performance targets (Framework 13-class laptop, integrated graphics)

| Metric | Target |
|---|---|
| Cold start to empty grid (GUI) | < 300 ms |
| Cold start (TUI) | < 100 ms |
| Open 100 MB CSV (1M × 20 numeric) | first paint < 1 s (progressive), fully loaded < 4 s |
| Open 50 MB `.xlsx` | < 5 s |
| Save 50 MB `.xlsx` | < 5 s |
| Incremental recalc after one edit in a 100k-formula model | < 50 ms typical |
| Full recalc, 1M formulas | < 5 s on 8 threads |
| Keystroke to paint | < 16 ms |
| Scroll | 60 fps sustained at any sheet size |
| Memory, 1M × 20 numeric | < 1.5 GB resident |
| Theme reload | < 100 ms, no flicker, no lost edit state |
| Inline completion (local `fast` model) | first token < 300 ms; typing never blocks |
| Natural-language plan | shown < 3 s local, < 5 s cloud; cancellable |
| AI functions | batched ≥ 50 cells per request; recalculation of non-AI cells unaffected |
| Workbook card, 1M-cell workbook, `columns` level | < 200 ms |

Targets are CI gates with a 10 % regression budget (§14).

### 12.2 Reliability

- Autosave and crash recovery (§6.9). A crash during save never corrupts the original: write to temp, fsync, rename.
- Undo history is durable across autosave cycles within a session.
- A malformed config or theme never prevents launch; the app falls back one layer and reports.
- File readers are fuzzed continuously; any panic on input is a P1 bug.

### 12.3 Security and privacy

- No network by default; `[network] enabled` and a per-function allowlist gate tier-2 functions and external links, with a per-workbook allowlist stored in the file's custom part and re-confirmed on a new machine.
- No telemetry, no update checks (the package manager handles updates), no crash uploads. Diagnostics go to `~/.local/state/omacell/logs/` with rotation.
- Embedded scripts: sandboxed by default, trust explicit and per file hash, never prompted on open; VBA is preserved inert.
- Parsers enforce decompression ratio and size limits, disable XML external entities, and reject path traversal in zip entries.
- Sheet/workbook "passwords" are documented as compatibility features, not protection. Real confidentiality is the filesystem's job (Omarchy ships full-disk encryption).
- AI: off by default; loopback-only until a cloud provider is explicitly added; privacy level and redaction enforced before any payload is built; secrets by environment variable or command only; mutations only through changesets; no tool available to a model can alter policy, run scripts, or reach the network through the application; a prompt-injection suite runs in CI (§8.7, §14).

### 12.4 Accessibility

Keyboard-complete; AT-SPI exposure of cell address, value, formula presence, and errors; respects the desktop `reduced-motion` and high-contrast hints; focus always visible; minimum UI text size 9 px following the Omarchy knob's floor; color is never the only carrier of meaning (errors also carry a glyph; conditional icons have text alternatives in the panel).

### 12.5 Portability

Runs on any Linux with Wayland (X11 through XWayland, best effort). Omarchy-specific behavior activates only when the Omarchy theme directory is found; otherwise the portal color-scheme and fontconfig defaults apply. No dependency on Quickshell or Hyprland at runtime.

## 13. Excel compatibility matrix

| Excel capability | v1 | v1.x | Later / never |
|---|---|---|---|
| A1/R1C1, relative/absolute, 3-D, whole row/col refs | ● | | |
| Dynamic arrays, spill, `@`, `LET`, `LAMBDA` + helpers | ● | | |
| Function library | Tier 0 (~260) | Tier 1 (~200) | Tier 2 behind network flag; cube/RTD never |
| Number format codes | ● full | | |
| Styles, borders, fills, rich text | ● | gradients render (preserved in v1) | |
| Conditional formatting (all rule types) | ● | | |
| Data validation | ● | | |
| Sort, AutoFilter, advanced filter | ● | | |
| Tables, structured refs, totals, slicers | ● (no slicers) | slicers | |
| Pivot tables | ● core | show-as, calculated fields | Power Pivot never |
| Charts | line, bar/column, area, pie, scatter, bubble, combo, histogram, sparklines | trendlines, error bars, waterfall, box | 3-D chart styles never |
| Comments (threaded) and notes | ● | | |
| Hyperlinks, protection flags | ● | | |
| Freeze/split, outline/grouping | ● | | |
| Goal Seek / Data Tables / Scenarios / Solver | Goal Seek | Data Tables, Scenarios | Solver as plugin |
| Print, page setup, PDF | ● | | |
| Flash Fill | | ● | |
| Macros | Lua recorder | | VBA execution never |
| Power Query / data model | | | never (CSV/JSON/Parquet import + Lua instead) |
| Co-authoring | | | never (v1 scope) |
| `.xlsx`/`.xlsm` read/write | L1–L2 + L3 preserve | | |
| `.xls` | native BIFF read | | write never |
| `.ods` | read | write | |
| AI functions (`AI`, `AI.EXTRACT`, …) | Omacell extension; cached values written to `.xlsx` | | Excel's `COPILOT()` imported inert (mapping to `AI()` open) |
| Copilot-style assistance | palette plans, formula assist, import assist, audit | in-app agent panel | — |

## 14. Testing and quality strategy

- **Function conformance corpus.** Thousands of `(formula, expected)` cases per function drawn from documented Excel behavior, including edge cases (empty, errors, text-numbers, negative zero, dates at boundaries). Cross-checked against LibreOffice headless where both should agree; disagreements are triaged and recorded.
- **`.xlsx` round-trip corpus.** Real-world files (with permission) plus generated ones. Open → save → semantic diff must be empty at L1 and L2; L3 parts must be byte-identical. Files saved by Omacell are reopened by LibreOffice in CI as a second reader.
- **CSV corpus.** Encodings, delimiters, quoting edge cases, locale separators, ragged rows, 1 GB progressive load.
- **Theme contract tests.** Every built-in Omarchy theme's `colors.toml` (and a set of community themes) is run through the template and the direct mapping; the two must agree; contrast rules must hold; screenshots are compared to goldens per theme and per fractional scale.
- **Keymap tests.** Every default binding resolves to a registered command; `omacell keys check` is run against Omarchy's default `bindings.lua` to catch new conflicts on each Omarchy release.
- **Config tests.** Schema validation, layering precedence, migration from every prior schema version, live-reload with invalid intermediate states.
- **Fuzzing.** Formula parser, number-format parser, xlsx/ods/csv readers, IPC decoder — continuous, with crash artifacts kept.
- **Performance gates.** The §12.1 table as CI benchmarks on a fixed reference machine; regressions beyond budget block merge.
- **Omarchy release tracking.** A CI job installs Omarchy's stable, RC, and edge channels in VMs, runs `omacell setup omarchy`, switches themes, changes fonts and text size, picks a default agent, and asserts the integration behaviors in §7 and §8.5.
- **AI contract fixtures and evals.** Required CI stays network-free with
  explicitly synthetic fixtures that exercise plan validation/effects, formula
  execution, import overlays, audit parsing/scoring, and injection containment;
  these checks are not reported as model-quality scores. Nightly, the same
  prompts and independently declared oracles run against a small local model
  to measure plan accuracy/effect equivalence, formula/import success, audit
  precision/recall, and prompt-injection drift.
- **Prompt-injection suite.** Cells, headers, comments, and CSV inputs containing instruction-shaped text are pushed through every AI feature; the assertion is zero unexpected commands and zero policy changes.
- **Changeset invariants.** Property tests assert that in review mode no AI-origin mutation reaches the workbook without an applied changeset, that apply and revert are exact inverses, and that autopilot scope and operation caps hold.
- **MCP and skill contract tests.** An MCP client exercises every tool and resource against a fixture workbook, and the skill's instructions are checked against the CLI's actual `--help` output so the two cannot drift apart.

## 15. Roadmap

| Milestone | Scope | Exit criteria |
|---|---|---|
| **M0 — Spikes** (4–6 weeks) | ADR-001 toolkit spike; ADR-002 engine evaluation; `.xlsx` L1 round-trip prototype; theme hot-reload prototype | Decisions recorded; 1M-row scroll at 60 fps demonstrated; Tokyo Night → Catppuccin Latte switch live |
| **M1 — Grid and formulas** (v0.1) | Core model, parser, ~150 Tier-0 functions, classic keymap, `.xlsx` L1 read/write, CSV in/out with preview, GUI + TUI basics, theming, config layering, `convert`/`eval`/`query` CLI, MCP server, shipped skill, `omacell agent` hand-off | Use cases 1 and 4 (§5.2) work end to end; the default Omarchy agent can query and edit a workbook through the skill |
| **M2 — Daily driver** (v0.2) | `.xlsx` L2, styles/number formats complete, sort/filter/tables/validation/conditional formatting, find/replace/goto, freeze/split, undo panel, autosave/recovery, modal keymap, `init.lua` + custom functions, IPC, `omacell setup omarchy`, provider abstraction, workbook card, changeset review, natural-language palette, formula explain/generate | A week of real work without opening Calc; use case 6 works |
| **M3 — Analysis** (v0.3) | Pivot tables, core charts, Goal Seek, comments, protection, print/PDF, ODS read, Tier 1 functions begun, AI functions with cache and batching, import assistant, AI audit, inline completion | Use cases 2 and 3 work end to end; an `AI.FILL` column survives save, reopen, and Excel |
| **M4 — Ecosystem** (v0.4) | Plugins, macro recorder, Parquet, native `.xls` reader, menu/hook/OCR integrations, Python bridge, localization, in-app agent panel and skills, diagnosis hand-off, redaction detectors, usage reporting, vision `render` tool | Third-party plugin published; use case 7 works: an agent audits a seeded workbook and its changeset is reviewed in the GUI |
| **1.0** | Performance gates met, Omarchy channel CI green, manual complete, AUR stable | Public release |

## 16. Risks and open decisions

### 16.1 Risks

| Risk | Mitigation |
|---|---|
| `.xlsx` fidelity is unbounded | Fidelity levels with measured coverage; L3 preserve-and-re-emit; corpus-driven prioritization |
| Function semantic drift from Excel | Conformance corpus; LibreOffice cross-check; document known differences |
| Toolkit accessibility/IME gaps | M0 spike gates; a11y and IME are exit criteria, not polish |
| Omarchy integration churn (4.0 already moved the theme path) | Multi-path resolution; channel-tracking CI; integration isolated in `conf/` adapters |
| Scope creep toward "all of Excel" | §2.2 non-goals and §13 matrix are the backlog fence |
| Naming/trademark | Working name is a placeholder; decide before first public release |
| Single-maintainer bus factor | Rust + boring dependencies; generated docs; corpora as executable spec |
| Model output quality varies by provider and model | Grounding on the command registry and workbook card; plans and formulas validated by the parser and evaluator before proposal; offline evals per model slot |
| Prompt injection through cell content | Data fencing, changeset-only mutation, policy tools unavailable to models, standing injection suite (§8.7) |
| Cost and latency surprises with cloud providers | Local-first defaults, budgets and confirmations (§8.3), usage visible in the status line |
| Agent-harness churn (ten CLIs; skill and MCP conventions still moving) | One skill format, one MCP server, `omarchy default agent` as the single hand-off point; per-harness checks in the Omarchy CI job |

### 16.2 Open decisions

1. Name: *Omacell* is the working name; still open are Omarchy's blessing for the "Oma-" prefix, a trademark clearance search, and handles/domains (`omacell.com` and the main social handles are already held by unrelated parties).
2. ADR-001 toolkit and ADR-002 engine, after M0.
3. Default cell font: monospace (Omarchy-native) vs. proportional (Excel-native). Current default: monospace.
4. License: **MIT**, decided 31 Aug 2026 to match Omarchy.
5. Whether to propose Omacell as an Omarchy default app or remain an ecosystem package. Current lean: ecosystem package until 1.0.
6. Whether `.omc` should be a first-class save target in the UI or remain an export.
7. Whether to map Excel's `COPILOT()` to `AI()` on import. Current lean: import inert, offer a one-key conversion.
8. Whether the in-app agent panel ships in 1.0 or after, given that the external-agent path (skill + MCP + hand-off) covers most agentic use. Current lean: after, unless M2 shows the review overlay carries most of the cost anyway.
9. Default privacy level for non-loopback providers: `schema` (current) vs `sample`.

---

## Appendix A — Default keymaps

### A.1 Classic (Excel-compatible) — selected bindings

| Key | Action | Key | Action |
|---|---|---|---|
| Arrows / `Ctrl+Arrow` | Move / jump to data edge | `Shift+Arrow` / `Ctrl+Shift+Arrow` | Extend selection |
| `Home` / `Ctrl+Home` / `Ctrl+End` | Column A / A1 / last used | `PgUp` `PgDn` / `Alt+PgUp` `Alt+PgDn` | Page vertical / horizontal |
| `Ctrl+PgUp` / `Ctrl+PgDn` | Previous / next sheet | `Ctrl+G`, `F5` | Go To |
| `Tab` / `Shift+Tab` | Right / left | `Enter` / `Shift+Enter` | Commit and move down / up |
| `F2` | Edit in cell | `Esc` | Cancel edit / close panel |
| `F4` | Cycle reference anchoring (editing); repeat last action | `F9` / `Shift+F9` / `Ctrl+Alt+F9` | Recalc all / sheet / full rebuild |
| `Ctrl+Enter` | Fill selection with entry | `Alt+Enter` | Line break in cell |
| `Ctrl+D` / `Ctrl+R` | Fill down / right | `Ctrl+E` | Flash Fill (v1.x) |
| `Ctrl+;` / `Ctrl+Shift+;` | Insert date / time | `Ctrl+'` / `Ctrl+Shift+"` | Copy formula / value from above |
| `Ctrl+Z` / `Ctrl+Y` | Undo / redo | `Ctrl+X` `Ctrl+C` `Ctrl+V` | Cut / copy / paste |
| `Ctrl+Alt+V` | Paste Special panel | `Delete` | Clear contents |
| `Ctrl+-` / `Ctrl+Shift+=` | Delete / insert cells, rows, columns | `Ctrl+9` `Ctrl+0` / `Ctrl+Shift+9` `Ctrl+Shift+0` | Hide / unhide rows, columns |
| `Ctrl+Space` / `Shift+Space` | Select column / row | `Ctrl+A` | Region, then all |
| `F8` | Extend mode | `Shift+F8` | Add to selection |
| `Ctrl+1` | Format panel | `Ctrl+B` `Ctrl+I` `Ctrl+U` | Bold / italic / underline |
| `Ctrl+Shift+~ ! @ # $ % ^` | General / number / time / date / currency / percent / scientific | `Ctrl+Shift+&` / `Ctrl+Shift+_` | Outline border on / off |
| `Ctrl+Shift+L` | Toggle AutoFilter | `Ctrl+T` | Create table |
| `Alt+=` | AutoSum | ``Ctrl+` `` | Show formulas |
| `Ctrl+F` / `Ctrl+H` | Find / replace | `Shift+F2` | Note / comment |
| `Ctrl+K` | Hyperlink | `Ctrl+F3` / `F3` / `Ctrl+Shift+F3` | Name manager / paste name / create from selection |
| `Alt+Shift+→` / `Alt+Shift+←` | Group / ungroup | `Ctrl+Shift+E` | Explain error |
| `Ctrl+N` `Ctrl+O` `Ctrl+S` `F12` `Ctrl+W` `Ctrl+P` | New / open / save / save as / close / print | `F11` | Chart from selection |
| `Ctrl+Shift+P` | Command palette | `F1` | Keys overlay |
| `Ctrl+Shift+A` | AI: natural-language plan (displaces Excel's argument-name insert, which formula hints cover) | `Ctrl+Shift+X` | AI assist on cell: explain / generate / fix / refactor |
| `Ctrl+Shift+R` | Changeset review panel | `Tab` (while editing) | Accept inline completion |
| `Ctrl+Alt+=` / `Ctrl+Alt+-` / `Ctrl+Alt+0` | Zoom in / out / reset (Excel's `Ctrl+-` delete and `Ctrl+0` hide stay intact) | `Ctrl+Shift+U` | Expand / collapse formula bar |

### A.2 Modal (Vim-style, opt-in) — selected bindings

| Mode | Key | Action |
|---|---|---|
| Normal | `h j k l`, counts | Move |
| Normal | `gg` / `G` / `0` / `$` / `H` `M` `L` | Top / bottom / first column / last used column / screen top-middle-bottom |
| Normal | `Ctrl+f` `Ctrl+b` `Ctrl+d` `Ctrl+u` | Page / half-page |
| Normal | `w` / `b` | Next / previous data edge |
| Normal | `i` / `a` / `=` / `Enter` | Edit cell / append / start formula / edit |
| Normal | `x` / `dd` | Clear cell / clear row contents |
| Normal | `dr` `dc` / `ir` `ic` / `yr` `yc` / `yy` `p` `P` | Delete row/col · insert row/col · yank row/col · yank cell · paste after/before |
| Normal | `u` / `Ctrl+r` / `.` | Undo / redo / repeat |
| Normal | `v` / `V` / `Ctrl+v` | Visual range / row / column |
| Normal | `/` `n` `N` | Search / next / previous |
| Normal | `gt` / `gT` / `<n>gt` | Next / previous / nth sheet |
| Normal | `zf` / `zs` / `zz` | Freeze at cursor / split / center |
| Normal | `:` | Command line (`:w`, `:q`, `:e`, `:sort`, `:fmt`, `:goto`, `:set`, `:fn`, `:source`) |
| Normal | `<leader>` (default `Space`) | User chord prefix (e.g. `<leader>p` pivot builder) |
| Normal | `<leader>a` / `:ai <text>` | AI: natural-language plan |
| Normal | `<leader>x` | AI assist on cell (explain / generate / fix / refactor) |
| Normal | `<leader>c` | Changeset review (`a` accept, `r` reject, `A`/`R` all) |
| Normal | `<leader>g` | Hand workbook to the default Omarchy agent |
| Visual | `d` `y` `c` `>` `<` `:` | Delete / yank / change / indent / outdent / command on range |
| Insert | `Esc` / `Ctrl+[` | Return to Normal; `Enter` commits and stays in Normal |

Full maps: `/usr/share/omacell/default/keys/classic.toml` and `keys/modal.toml`.

## Appendix B — `config.toml` (complete, with defaults)

```toml
schema = 1

[appearance]
cell_font        = "monospace"   # fontconfig alias; Omarchy points it at Style > Font
cell_font_size   = 11            # pt; cells scale with zoom
ui_font          = "monospace"
ui_font_size     = "system"      # follows Omarchy display text size; or a number in pt
grid_lines       = true
grid_line_style  = "solid"       # solid | dotted | none
row_height       = "auto"        # or a number in pt
column_width     = 8.43          # Excel default, in characters
cell_padding     = 3             # px at 1x
cursor_style     = "outline"     # outline | block | underline
selection_style  = "fill"        # fill | outline
show_formula_bar = true
show_status_line = true
show_sheet_tabs  = true
sheet_tabs_position = "top"      # top | bottom
corner_style     = "system"      # system | rounded | sharp
zebra_rows       = false
enforce_contrast = true
animation        = "system"      # system | on | off

[behavior]
enter_moves            = "down"  # down | right | none
autocomplete           = true
autocorrect            = false
formula_hints          = true
reference_style        = "A1"    # A1 | R1C1
default_sheets         = 1
date_system            = 1900    # 1900 | 1904 (new workbooks)
precision_as_displayed = false
smart_paste            = true    # detect delimited text on paste
fill_prompt            = true    # show fill-options after a fill

[calc]
mode             = "automatic"   # automatic | automatic_except_tables | manual
threads          = "auto"
iterative        = false
max_iterations   = 100
max_change       = 0.001
volatile_on_open = true

[locale]
language                 = "system"
decimal_separator        = "system"
thousands_separator      = "system"
list_separator           = "system"
date_format              = "system"
currency                 = "system"
first_weekday            = "system"
localized_function_names = false

[files]
default_format        = "xlsx"   # xlsx | omc
autosave_interval     = 60       # seconds; 0 disables
keep_backups          = 0
follow_external_links = false
[files.csv]
delimiter      = "auto"
encoding       = "auto"
type_inference = "conservative"  # conservative | aggressive | none
[files.xlsx]
preserve_unknown_parts = true

[session]
restore           = true
recent_files      = 20
workspace_binding = false        # remember which Hyprland workspace each window was on

[layout]
panel_side          = "right"    # right | left | bottom
panel_width         = 360        # px at 1x
formula_bar_lines   = 1
compact_below_width = 720        # px; collapse chrome below this
status_line         = ["mode", "cell", "stats", "calc", "dirty", "theme", "zoom"]
menu_bar            = false

[integrations]
omarchy              = "auto"    # auto | on | off
notifications        = "recovery_only"  # all | recovery_only | off
menu_entries         = true      # offered by `omacell setup omarchy`
libreoffice_fallback = false     # deprecated compatibility key; .xls import is native
ocr_paste            = true

[network]
enabled         = false
allow_functions = []             # e.g. ["WEBSERVICE"]
proxy           = ""

[scripting]
enabled          = true
trusted_dirs     = ["~/.config/omacell"]
embedded_scripts = "sandbox"     # sandbox | ask | deny

[ai]
enabled        = false           # nothing runs and nothing is sent until this is true
status_segment = true

# Providers are wire protocols, not vendors (ADR-005). `omacell ai setup` writes the
# local ones it detects; cloud blocks are added by hand and never hold a plaintext key.
[ai.providers.ollama]
kind     = "openai_compatible"
endpoint = "http://localhost:11434/v1"
local    = true
[ai.providers.lmstudio]
kind     = "openai_compatible"
endpoint = "http://localhost:1234/v1"
local    = true
# [ai.providers.anthropic]
# kind       = "anthropic"
# endpoint   = "https://api.anthropic.com"
# secret_env = "ANTHROPIC_API_KEY"   # or: secret_cmd = "op read op://Private/Anthropic/credential"

[ai.models]                      # task slots -> "provider:model" (example values)
fast    = "ollama:qwen2.5-coder:7b"
default = "ollama:qwen2.5:14b"
strong  = ""                     # empty = falls back to default
agent   = ""
vision  = ""

[ai.privacy]
send              = "schema"     # schema | sample | full (loopback providers default to full)
local_full        = true
suggest_redaction = true
log_content       = false

[ai.functions]
auto                    = true   # re-query when inputs change; false = only on ai.refresh
batch_size              = 50
max_cells_per_recalc    = 500    # above this, confirm with an estimate first
max_requests_per_minute = 60
max_tokens_per_request  = 4096
keep_stale              = true
refresh_on_full_recalc  = false
xlsx_export             = "formulas"   # formulas | values

[ai.completion]
mode     = "auto"                # auto (on only if the fast model is local) | on | off
debounce = 250                   # ms

[ai.agent]
review            = "always"     # always | autopilot_opt_in
autopilot_scope   = "sheet"      # sheet | range | workbook
autopilot_max_ops = 200
diagnose_offers   = true
panel             = true
skills_dir        = "~/.config/omacell/ai/skills"

[charts]
palette      = "theme"
default_type = "column"
line_width   = 2
font         = "ui"

[tui]
unicode_borders = true
truecolor       = "auto"
mouse           = true
graphics        = "auto"         # auto | sixel | kitty | off

[keys]
file = "keys.toml"               # model and bindings live there
```

## Appendix C — Theme template (`omacell.toml.tpl`)

Installed by `omacell setup omarchy` to `~/.config/omarchy/themed/omacell.toml.tpl`; Omarchy renders it into `<active-theme>/omacell.toml` on every theme switch. The built-in mapping in the binary is identical, so editing this file is how a user changes the *mapping*.

```toml
# Omacell color roles — rendered from the active Omarchy theme's colors.toml.
# Placeholders: {{ key }}, {{ key_strip }}, {{ key_rgb }}, {{ mix a b 20% }}.
mode = "{{ mode }}"

[surfaces]
background        = "{{ background }}"
surface           = "{{ lighter_background }}"     # panels, formula bar
header_background = "{{ dark_background }}"        # row/column headers
popup_background  = "{{ darker_background }}"

[text]
foreground        = "{{ foreground }}"
muted             = "{{ muted }}"                  # empty-cell hints, placeholders
header_foreground = "{{ dark_foreground }}"
bright            = "{{ bright_foreground }}"

[structure]
grid_line    = "{{ mix background foreground 12% }}"
pane_divider = "{{ mix background foreground 35% }}"
frozen_edge  = "{{ accent }}"

[state]
cursor           = "{{ accent }}"
selection        = "{{ selection }}"
selection_border = "{{ accent }}"
active_header    = "{{ accent }}"
hover            = "{{ mix background foreground 6% }}"
stale            = "{{ mix background muted 50% }}"  # hatching for not-yet-recalculated cells

[semantic]
error   = "{{ red }}"
warning = "{{ color3 }}"
success = "{{ color2 }}"
info    = "{{ color4 }}"
link    = "{{ blue }}"

[references]   # cycle used to colorize ranges while editing a formula
colors = ["{{ color4 }}", "{{ color2 }}", "{{ color5 }}", "{{ color3 }}",
          "{{ color6 }}", "{{ color1 }}", "{{ accent }}", "{{ color7 }}"]

[charts]
palette = ["{{ accent }}", "{{ color2 }}", "{{ color3 }}", "{{ color5 }}",
           "{{ color6 }}", "{{ color1 }}", "{{ color4 }}", "{{ color7 }}"]
axis     = "{{ dark_foreground }}"
gridline = "{{ mix background foreground 10% }}"

[conditional]  # defaults for new color scales / data bars
scale_low  = "{{ red }}"
scale_mid  = "{{ color3 }}"
scale_high = "{{ color2 }}"
data_bar   = "{{ accent }}"
```

Companion hook, installed to `~/.config/omarchy/hooks/theme-set.d/omacell`:

```sh
#!/bin/sh
# Omarchy runs this after a theme change; $1 is the theme name.
exec omacell ipc theme.reload --all --quiet
```

## Appendix D — Function tiers (summary)

Full lists are generated from the registry (`omacell fn list --tier 0 --json`). Counts are approximate.

| Category | Tier 0 (v1) | Tier 1 (v1.x) | Tier 2 / never |
|---|---|---|---|
| Math & trig | 60 | 10 (`SERIESSUM`, `MULTINOMIAL`, …) | — |
| Statistical | 45 descriptive | 90 distributions & regression | — |
| Text | 35 (+ `REGEXTEST/EXTRACT/REPLACE` extension) | `BAHTTEXT`, `PHONETIC`, DBCS family | — |
| Logical | 10 | — | — |
| Lookup & reference | 35 | `GETPIVOTDATA`, `HYPERLINK`, `FORMULATEXT` | `RTD` never |
| Date & time | 25 | — | — |
| Financial | 20 | 35 (bonds, T-bills, `DURATION`, `ACCRINT`) | — |
| Information | 15 | `INFO`, `SHEET(S)` | — |
| Engineering | 12 (`CONVERT`, bases) | 30 (Bessel, complex, `ERF`) | — |
| Database | — | 12 (`D*`) | — |
| Web / cube | — | — | `WEBSERVICE`, `FILTERXML`, `ENCODEURL`, `IMAGE` behind `[network]`; `CUBE*` never |
| Lambda helpers | 8 | — | — |
| Array manipulation | 15 | — | — |
| AI (extension namespace, §8.3) | `AI`, `AI.EXTRACT`, `AI.CLASSIFY`, `AI.FILL`, `AI.TABLE`, `AI.TRANSLATE` — live only with a provider | user-registered via `omacell.ai.fn` | — |

## Appendix E — `.omc` text workbook (sketch)

Line-oriented, UTF-8, one record per line, tab-separated, `#` comments. Designed for `git diff`, `grep`, and generation by scripts. Not a replacement for `.xlsx`.

```
omc 1
book	date_system=1900	calc=automatic
name	TaxRate	=Inputs!$B$3
style	1	font=monospace;bold	numfmt=#,##0.00	fill=#1a1b26
sheet	Inputs	cols=A:8.43,B:12	freeze=A2
cell	Inputs!A1	Rate	s=1
cell	Inputs!B3	0.07	fmt=0.00%
sheet	Model
cell	Model!A1	=SEQUENCE(12)
cell	Model!B1	=A1#*TaxRate
cf	Model!B1:B12	rule=cell_value	op=gt	value=1	style=2
validation	Inputs!B3	type=decimal	between	0	1
comment	Inputs!B3	author=shea	Confirm with finance
```

Values are stored as typed literals (`0.07`, `"text"`, `TRUE`, `#N/A`), formulas as `=…`; cell order is row-major so diffs are stable. Binary parts (images, unknown `.xlsx` parts) are not representable; converting `.xlsx` → `.omc` lists what was dropped.

## Appendix F — Glossary

| Term | Meaning |
|---|---|
| Active theme | The Omarchy theme directory currently in effect (`~/.local/state/omarchy/current/theme` on 4.x) |
| Autopilot | Opt-in, scoped, capped mode in which an AI origin applies changesets without per-item review (§8.6) |
| Changeset | An ordered, invertible list of command-bus commands with origin and status; the only way a model mutates a workbook (§8.6) |
| Command bus | The registry of named, JSON-argument commands shared by all front-ends |
| Fidelity level (L1–L3) | Degree of `.xlsx` round-trip guarantee (§6.9) |
| MCP | Model Context Protocol; `omacell mcp` exposes the command bus as tools and resources for any agent harness (§8.5) |
| Provider / slot | A configured model endpoint (`[ai.providers.*]`) and the task role it is assigned to (`fast`, `default`, `strong`, `agent`, `vision`) |
| Role | A named color slot in Omacell's UI, mapped from theme palette keys (Appendix C) |
| Skill | A `SKILL.md` directory teaching an agent how to use a tool; one format shared by Omarchy, the coding agents, and Omacell's in-app agent (§8.8) |
| Spill | A dynamic-array result occupying cells beyond the formula's own |
| Template (`.tpl`) | An Omarchy file with `{{ placeholders }}` rendered from `colors.toml` on theme switch |
| Tier | Function-library delivery grouping (Appendix D) |
| Workbook card | The budgeted, redactable JSON description of a workbook given to models (§8.2) |

---

*End of specification.*
