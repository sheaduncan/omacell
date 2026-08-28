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

LibreOffice disagreements discovered by `scripts/lo-crosscheck.py` should be
appended here rather than papered over in the corpus.
