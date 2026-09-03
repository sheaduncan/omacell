# Known differences from Excel

Documented here so WP-05a/b/c and the LibreOffice cross-check script have a
single place to record intentional divergences. Rows cite the behaviour they
encode.

| Topic | Excel | Omacell | Package |
|---|---|---|---|
| `NOW`/`TODAY` clock | Wall clock per session | One sample per recalc pass; tests inject a serial | WP-05F |
| `RAND` | Non-deterministic | Pass-stable splitmix from an injected or sampled nonce, mixed with cell and pass | WP-05F |
| Numeric-text comparison | Does not coerce in `=` | Same (WP-04 coerce) | WP-04 |
| `PERMUTATIONA(0,0)` | Excel `#NUM!` | `1` (`0^0` as combinatoric empty product) | WP-05a |
| LibreOffice CSV error tokens | `#NUM!` / `#VALUE!` / `#N/A` | `Err:502` / `Err:504` / `Err:511` / `Err:539` in headless CSV | WP-05a |
| LibreOffice dotted / post-2007 names | Excel evaluates `STDEV.S`, `SWITCH`, `XOR`, `ACOT`, … | XLSX importer needs `_xlfn.` and still `#NAME?`s some names (`ISO.CEILING`, `ISOMITTED`) | WP-05a |
| LibreOffice `TYPE(TRUE)` | `4` (logical) | `1` (number) | WP-05a |
| LibreOffice array logicals in `SUM`/`COUNT` | Skip logicals in arrays/ranges | Often includes `TRUE` as 1 | WP-05a |
| `LEN`/`MID`/`LEFT`/`RIGHT` of astral scalars (e.g. `😀` U+1F600) | Excel for Windows (365 included) counts UTF-16 code units, so `LEN` = 2 and `MID` can emit a lone surrogate | Unicode scalar values (`LEN` = 1); slices never split a code point | WP-05b |
| `UPPER("ß")` | Stays `ß` | Same (one-to-one mapping; not Unicode `SS`) | WP-05b |
| `REGEXTEST`/`REGEXEXTRACT`/`REGEXREPLACE` | Excel 365; LibreOffice has no equivalent names | Implemented with the `regex` crate (linear-time, 256-char pattern / 1 MiB compile cap) | WP-05b |
| LibreOffice CSV of date serials | LO often emits a locale date string (`01/01/2024`) or `Err:502` | Omacell corpus expects the numeric serial / Excel error token | WP-05b |
| LibreOffice array-lifting | Implicit intersection: first element only | Dynamic-array lift (`{2,3}`) | WP-05b |
| LibreOffice CSV double unary boolean | Keeps `--TRUE` as logical `TRUE` | Coerces to numeric `1` per spec F-3.5 | WP-04 |
| LibreOffice `YEAR(0)`/`YEAR(1)` | 1899 (LO epoch 1899-12-30) | 1900 (Excel January 0 / 1 Jan 1900) | WP-05b |
| LibreOffice `YEARFRAC` reversed dates | Absolute value | Signed (Excel) | WP-05b |
| LibreOffice `LEN(TRUE)` | 1 (TRUE as number) | 4 (TRUE as text) | WP-05b |
| LibreOffice `REGEX*` / `ARRAYTOTEXT` / `VALUETOTEXT` / `TEXTSPLIT` | `#NAME?` or no `_xlfn` mapping | Implemented | WP-05b |
| Invalid `MAKEARRAY` / stacking / wrapping shapes (`0`, out of grid, overflow) | Excel 365 uses `#CALC!` for some zero-size cases | `#NUM!` for all invalid shapes (checked before allocation) | WP-05c |
| `RANDARRAY` | Non-deterministic | Pass-stable splitmix from the injected or sampled nonce, mixed with cell, pass, call path, and array index | WP-05c |
| `RATE` / `IRR` / `XIRR` solvers | Excel Newton; 20 iterations (`RATE`/`IRR`); undocumented `XIRR` cap | Newton–Raphson; `RATE`/`IRR` 20 iters; `XIRR` 100 iters; success `\|f\| < 1e-8`; else `#NUM!`; default guess `0.1`. Residuals with `\|rate\| < 1e-12` snap to `0` | WP-05c |
| Approximate `VLOOKUP`/`HLOOKUP`/`MATCH`/`LOOKUP` on unsorted data | Binary search (may return a “wrong” match) | Same binary search; not replaced with a linear scan | WP-05c |
| LibreOffice `_xlfn.*` helpers in headless CSV | Computed result | Often `#NAME?` (script classifies as known) | WP-05c |
| LibreOffice `EFFECT`/`NOMINAL` CSV | `0.1025` | `10.25%` (script compares numerically) | WP-05c |
| Oversize `SEQUENCE` | `#NUM!` before allocation | LibreOffice CSV may return `1` | WP-05c |
| Legacy multi-cell CSE range size | May occupy the Excel worksheet grid | Fixed-range materialization is capped at 1,000,000 cells to bound hostile files | WP-04 / WP-09 |
| AI function inside a legacy CSE range | No native Excel equivalent | Formula-mode XLSX export rejects this combination because the frozen v1 AI bridge cannot carry fixed-range metadata; values-mode export flattens it safely | WP-04 / WP-23 |
| `INDEX` of an empty cell | blank | LibreOffice CSV may show `0` | WP-05c |
| Sort of formula cells | Cells move as units; relative refs adjust by the row/col delta of the move (same as copy/fill) | Same: `RewriteOp::Copy { drow, dcol }` after the permutation. Absolute refs stay. Notes/comments do not follow the sort. | WP-18 |
| `CONVERT` unknown/incompatible unit | `#N/A` | LibreOffice `Err:502` | WP-05c |
| `CONVERT` `lbm`→`g` | Microsoft factor `453.59237` | LibreOffice uses a slightly different mass factor | WP-05c |
| LibreOffice bit shifts at Excel limits | Shift amounts through ±53 are allowed, but input/results above `(2^48)-1` are `#NUM!` | Same; LibreOffice may return an out-of-range shift result | WP-05c |
| Pivot compact layout | Nested row fields indent in one column; outer keys get a group header row | Same: two spaces per depth, group header when an outer key changes; outline blanks repeated outer labels; tabular repeats them | WP-24a |
| Pivot Distinct Count | Excel 2013+ `x14:dataField pivotShowAs="distinctCount"` | Aggregates distinctly and writes the x14 extension; `subtotal="count"` remains the base attribute | WP-24a |
| Structural edits around pivots | Excel rewrites pivot source/output references as rows and columns move | Whole-row/column inserts and deletes, and cell shifts that cover a pivot range, rewrite source and output; partial bands that would split a pivot still error `pivot.struct` | WP-24a |
| Unsupported pivot extensions | Preserves calculated fields, slicers, and vendor extensions | Calculated fields are modeled; unchanged pivots re-emit original cache/table XML and relationships; a dirty pivot regenerates supported parts and drops unmodeled extensions | WP-24a |
| Goal Seek non-convergence | Status dialog; last trial remains | `converged: false` with a finite last trial; no error | WP-24 |
| Data Tables / Scenario Manager | Excel what-if tools | Deferred (v1.x); only Goal Seek in this package | WP-24 |
| Lua user stdlib | Full Lua 5.4 including `debug` | User scripts receive `mlua`'s full safe subset, including `io`, `os`, `package`, and Lua-module `require`; the unsafe `debug` library and native C-module loading remain unavailable under the workspace-wide `unsafe_code = "forbid"`. Embedded scripts additionally remove all file/process/module-loading entry points | WP-20 |
| `embedded_scripts = ask` | Prompt (never on open) | Non-interactive CLI treats `ask` as `deny` | WP-20 |

LibreOffice disagreements discovered by `scripts/lo-crosscheck.py` should be
appended here rather than papered over in the corpus. Corpus rows for an
intentional mismatch use `known difference` in their note so the script reports
them separately while still failing on every unexplained mismatch.
