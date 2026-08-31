# Known differences from Excel

Documented here so WP-05a/b/c and the LibreOffice cross-check script have a
single place to record intentional divergences. Rows cite the behaviour they
encode.

| Topic | Excel | Omacell | Package |
|---|---|---|---|
| Invalid `SEQUENCE` shape (`0`, negative, out of grid) | Excel 365 uses `#CALC!` for some zero-size cases | `#NUM!` for all invalid shapes (checked before allocation) | WP-05F / WP-05c |
| `NOW`/`TODAY` clock | Wall clock per session | One sample per recalc pass; tests inject a serial | WP-05F |
| `RAND` | Non-deterministic | Pass-stable splitmix from an injected or sampled nonce, mixed with cell and pass | WP-05F |
| Numeric-text comparison | Does not coerce in `=` | Same (WP-04 coerce) | WP-04 |
| IEEE `1.005` as `0.00` | May display `1.01` depending on binary rounding | `1.00` (15-digit then round) | WP-06 |
| `CELL` without a reference | Tracks the last changed cell | Uses the formula cell | WP-05a |
| `PERMUTATIONA(0,0)` | Excel `#NUM!` | `1` (`0^0` as combinatoric empty product) | WP-05a |
| `*IF` over array constants | Excel often `#VALUE!` (range required) | Walks array constants like ranges | WP-05a |
| LibreOffice CSV error tokens | `#NUM!` / `#VALUE!` / `#N/A` | `Err:502` / `Err:504` / `Err:511` / `Err:539` in headless CSV | WP-05a |
| LibreOffice dotted / post-2007 names | Excel evaluates `STDEV.S`, `SWITCH`, `XOR`, `ACOT`, … | XLSX importer needs `_xlfn.` and still `#NAME?`s some names (`ISO.CEILING`, `ISOMITTED`) | WP-05a |
| LibreOffice `TYPE(TRUE)` | `4` (logical) | `1` (number) | WP-05a |
| LibreOffice array logicals in `SUM`/`COUNT` | Skip logicals in arrays/ranges | Often includes `TRUE` as 1 | WP-05a |
| LibreOffice `CELL` without reference | Last changed cell (Excel) / formula cell (Omacell) | Uses the conversion sheet row of the formula | WP-05a |
| `LEN`/`MID`/`LEFT`/`RIGHT` of astral scalars (e.g. `😀` U+1F600) | Excel for Windows (365 included) counts UTF-16 code units, so `LEN` = 2 and `MID` can emit a lone surrogate | Unicode scalar values (`LEN` = 1); slices never split a code point | WP-05b |
| `CHAR`/`CODE` 128–159 | Windows-1252 (e.g. `CHAR(128)` is euro on US Windows) | Latin-1 / Unicode `U+0080`–`U+009F` | WP-05b |
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
| `INDEX` of an empty cell | blank | LibreOffice CSV may show `0` | WP-05c |
| Sort of formula cells | Cells move as units; relative refs adjust by the row/col delta of the move (same as copy/fill) | Same: `RewriteOp::Copy { drow, dcol }` after the permutation. Absolute refs stay. Notes/comments do not follow the sort. | WP-18 |
| `CONVERT` unknown/incompatible unit | `#N/A` | LibreOffice `Err:502` | WP-05c |
| `CONVERT` `lbm`→`g` | Microsoft factor `453.59237` | LibreOffice uses a slightly different mass factor | WP-05c |
| `BITLSHIFT` beyond 48 bits | `#NUM!` (`0..2^48-1`) | LibreOffice may return the untruncated shift | WP-05c |
| Pivot compact layout | Nested row fields indent in one column | Multi-field row keys are joined with ` \| ` in the snapshot | WP-24 |
| Pivot Distinct Count | Excel 2013+ distinct-count data field | Aggregates distinctly; OOXML export uses `subtotal="count"` with a Distinct-count caption | WP-24 |
| Structural edits around pivots | Excel rewrites pivot source/output references as rows and columns move | Row/column and cell-shift structural edits are rejected on sheets used by a pivot until reference rewriting is implemented | WP-24 / WP-24a |
| Unsupported pivot extensions | Preserves calculated fields, slicers, and vendor extensions | Supported fields are modeled; saving regenerates their cache/table parts and does not preserve unsupported pivot XML extensions | WP-24 / WP-24a |
| Goal Seek non-convergence | Status dialog; last trial remains | `converged: false` with a finite last trial; no error | WP-24 |
| Data Tables / Scenario Manager | Excel what-if tools | Deferred (v1.x); only Goal Seek in this package | WP-24 |
| Lua user stdlib | Full Lua 5.4 including `io`/`os` | `mlua` only exposes those via `unsafe`; both profiles start from the safe subset. Embedded also nils `io`/`os`/`package`/`debug`/`require`/`load*` | WP-20 |
| `embedded_scripts = ask` | Prompt (never on open) | Non-interactive CLI treats `ask` as `deny` | WP-20 |

LibreOffice disagreements discovered by `scripts/lo-crosscheck.py` should be
appended here rather than papered over in the corpus. Corpus rows for an
intentional mismatch use `known difference` in their note so the script reports
them separately while still failing on every unexplained mismatch.
