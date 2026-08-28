# WP-06 — Number formats, dates, locales, and the General algorithm

| | |
|---|---|
| Phase | 1 — Engine |
| Lane | A — Engine / core |
| Size | M (≈ 3–5) |
| Depends on | WP-01 |
| Unblocks | WP-05F, WP-05b, WP-07a, WP-08, WP-09, WP-11, WP-18 |
| Spec sections | §6.2 F-2.1, F-2.3, F-2.6, §6.12 F-12.2 |
| Where | `crates/core` (module `numfmt`, `dates`, `locale`) |

## Goal

Format any value with any Excel format code, in any supported locale, exactly as Excel would.

## Deliverables

- Format-code parser: sections (`pos;neg;zero;text`), conditions `[>=1000]`, colors `[Red]`, literals and quoted text, `,` scaling, `%`, scientific, fractions (`# ?/?`, fixed denominators), dates/times incl. elapsed `[h]:mm`, AM/PM, `@`, locale codes `[$-409]`, currency `[$€-407]`, `*` fill and `_` skip markers (returned as layout hints).
- `General` algorithm (≤ 11 significant digits, scientific fallback) plus `general_for_width(chars)` for width-dependent rendering; 15-significant-digit display rule.
- Date serial ↔ civil conversion for the 1900 system (Lotus bug: serial 60 = 29 Feb 1900) and the 1904 system; time fractions with rounding rules.
- Built-in format table (Excel `numFmtId` 0–49) for `.xlsx` mapping.
- Locale tables: decimal/thousands separators, date order, AM/PM strings, day/month names for `en-US` plus a data-driven path for others; API `format(value, fmt, locale) -> (text, color_hint, layout_hints)`.

## Implementation notes

- Keep this crate free of the workbook types beyond `Value`; the GUI/TUI/CLI all call it.

## Acceptance criteria

- [ ] Corpus `tests/corpus/numfmt/*.tsv` (≥ 400 rows: value, format, locale → text) passes, including negative zero, rounding at section boundaries, fractions, elapsed times.
- [ ] Date boundary corpus (serials 0, 59, 60, 61, 2958465; 1904 offsets) passes.
- [ ] `numFmtId` mapping tests for 0–49.
- [ ] Fuzz target `parse_numfmt` runs 10 minutes without panic.

## Tests

- Corpus table tests; `proptest` date round-trips; fuzz target.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-06.md` before writing code.
4. Create branch `wp/06-number-formats`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-06: Number formats, dates, locales, and the General algorithm`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
