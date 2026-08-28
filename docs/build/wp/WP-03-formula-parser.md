# WP-03 — Formula lexer, parser, printer, and reference rewriting

| | |
|---|---|
| Phase | 1 — Engine |
| Lane | A — Engine / core |
| Size | L (≈ 6–10) |
| Depends on | WP-01 |
| Unblocks | WP-04, WP-07a, WP-09 |
| Spec sections | §6.3 F-3.1, F-3.2, §6.5 F-5.2, §11.3, §14 |
| Where | `crates/core` (module `formula`: `lexer`, `parser`, `ast`, `printer`, `rewrite`, `deps`) |

## Goal

Parse the full Excel formula grammar into an AST that the evaluator, the editor, the rewriter, and the `.xlsx` reader all share.

## Deliverables

- Lexer: numbers (incl. `1e3`, `.5`, `5%`), strings with `""` escapes, booleans, error literals, identifiers, quoted sheet names, 3-D sheet ranges, structured-reference tokens (`Table[[#Headers],[Col]]`, `[@Col]`), operators, array constants `{1,2;3,4}`, whitespace-as-intersection, spill `#`, implicit intersection `@`.
- Pratt parser with Excel precedence: `:` > space > `,` > unary `-` > `%` > `^` > `*` `/` > `+` `-` > `&` > comparisons; function calls with omitted arguments; `MAX_FORMULA_LEN` enforced.
- AST (`Expr`) with byte spans for editor highlighting; reference nodes carry relative/absolute per axis; named, structured, spill, and 3-D references are distinct node kinds.
- R1C1 parse and print; canonical A1 printer (normalized spacing, upper-cased names, quoting rules) with `print(parse(x))` stability.
- Editor mode: error-tolerant parse returning a partial AST plus the first error position and expected-token set (for autocomplete and reference colorization).
- `rewrite`: copy with delta (relative refs move, absolute stay), move/cut (all refs to the moved range retarget), insert/delete rows/columns (ranges grow/shrink; fully deleted → `#REF!`), sheet rename, table rename.
- `deps`: extract referenced ranges, names, tables; flag volatility (`NOW`, `RAND`, `OFFSET`, `INDIRECT`, …) and dynamic references (`INDIRECT`, `OFFSET`) for the graph.

## Implementation notes

- Function names are validated against the registry at evaluation time, not here; the parser accepts any identifier followed by `(`.
- Locale: the parser works on the canonical form (`,` argument separator, `.` decimal). Localized entry is converted at the editor boundary (WP-14).

## Acceptance criteria

- [ ] Corpus `tests/corpus/formulas/valid.tsv` (≥ 500 formulas → canonical print) and `invalid.tsv` (≥ 100 with expected error offset) pass.
- [ ] Rewrite corpus: fill-down/right, cut-paste, insert/delete rows/cols with expected results (document each case with the Excel behavior it mirrors).
- [ ] Property: `print(parse(print(parse(x)))) == print(parse(x))`.
- [ ] Fuzz target `parse_formula` runs 10 minutes with no panic; parse throughput ≥ 100k formulas/s.

## Tests

- Corpus-driven table tests; `proptest` printer stability; `cargo-fuzz` target; criterion bench.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-03.md` before writing code.
4. Create branch `wp/03-formula-parser`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-03: Formula lexer, parser, printer, and reference rewriting`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
