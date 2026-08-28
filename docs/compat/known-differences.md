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

LibreOffice disagreements discovered by `scripts/lo-crosscheck.py` should be
appended here rather than papered over in the corpus. Corpus rows for an
intentional mismatch use `known difference` in their note so the script reports
them separately while still failing on every unexplained mismatch.
