# Corpus — numfmt

Excel number-format, date-serial, locale, and `General` fixtures for WP-06.

Each data row cites the Excel / ECMA behavior it encodes in the `note` column.
No third-party `.xlsx` files.

## Format fixtures

Columns: `value`, `format`, `locale`, `date_system`, `text`, `note`.

| File | What it covers |
|---|---|
| `general.tsv` | `General` / 11-char rule / scientific fallback / −0 / F-2.6 15 digits |
| `numbers.tsv` | `0#?`, grouping, `%`, comma scaling, rounding |
| `sections.tsv` | `pos;neg;zero;text`, conditions, colors, literals, locale/currency codes |
| `scientific.tsv` | `E+` / `E-` / engineering `##0.0E+0` |
| `fractions.tsv` | `# ?/?`, `# ??/??`, fixed denominators |
| `dates.tsv` | `y/m/d` tokens, names, 1900 + 1904 |
| `dates_boundary.tsv` | serials 0, 59, 60, 61, 2958465; 1904 offsets (F-2.1 Lotus quirk) |
| `times.tsv` | `h:mm`, AM/PM, elapsed `[h]:mm`, subseconds |
| `text_bool_error.tsv` | `@`, TRUE/FALSE coercion, errors ignore the format |
| `locales.tsv` | separators and day/month/AMPM names beyond `en-US` |
| `misc.tsv` | extra rounding, builtin samples, fill/skip |

`value` is a number literal, `TRUE`/`FALSE`, an Excel error (`#DIV/0!`), empty, or a double-quoted text string.

`date_system` is `1900` (Excel Windows / Lotus leap) or `1904`.

## Built-in `numFmtId` map

`builtin.tsv` columns: `id`, `locale`, `format`, `note`.

Ids 0–49. Language-neutral codes follow ECMA-376 18.8.30. Ids 5–8, 14, 22, 41–44 follow Excel’s locale-dependent forms; 23–26 are reserved and map to `General`; 27–36 keep the ECMA CJK strings so `.xlsx` ids round-trip.
