# Corpus — eval

Table-driven fixtures for WP-04 (evaluator, coercion, spill, graph, recalc).

Real `.omc` workbooks are WP-11. These files are an *omc-style* text dialect so
eval cases can be written before the native format exists.

| File | Kind | What it covers |
|---|---|---|
| `coerce.tsv` | TSV | F-3.5 coercion and comparison (self-contained formulas in A1) |
| `operators.tsv` | TSV | Arithmetic, concat, percent, power, unary, comparisons, array broadcast |
| `let_lambda.omc.txt` | omc-style | `LET`, `LAMBDA` IIFE/closures, named lambda, `ISOMITTED` |
| `spill.omc.txt` | omc-style | Dynamic-array spill, `#SPILL!` blocker, `A1#`, legacy CSE flag |
| `implicit_intersection.omc.txt` | omc-style | `@` and scalar read of a spill origin |
| `threed.omc.txt` | omc-style | 3-D refs (`Sheet1:Sheet3!A1`) |
| `structured.omc.txt` | omc-style | Structured table refs |
| `names.omc.txt` | omc-style | Defined names (range / constant / formula / named lambda) |
| `volatile.omc.txt` | omc-style | Volatile functions recalculate every pass |
| `cycles.omc.txt` | omc-style | Circular-ref detection (iteration off) |
| `cycles_iter.omc.txt` | omc-style | Iterative calculation converges within limits |

Each `note` cites the spec / Excel behaviour the row encodes.

## TSV columns (`formula`, `expected`, `note`)

The formula is stored in `A1` of a fresh workbook and fully recalculated.
`expected` is the canonical display of `A1` (and spill ghosts when the result
spills). See “Expected values” below.

## omc-style grammar

Blank lines and `#` comments are ignored. Commands:

```
sheet <name>
set <A1> <literal-or-formula>
set <Sheet>!<A1> <literal-or-formula>
flag <A1> array
table <Name> <A1:range> <col1> <col2> ...
name <Name> <referent>
settings iteration on max_iterations=<n> max_change=<f>
settings calc_mode automatic|manual|automatic_except_tables
expect <A1> <value>
expect_circular <A1> [<A1> ...]
expect_stale <A1> [<A1> ...]
expect_volatile <A1>
recalc                  # run a full recalc and check expects accumulated so far
```

`<referent>` for `name` is a range (`A1:B2`), a constant (`1`, `TRUE`, `"x"`),
or a formula starting with `=`.

The corpus runner may register a tiny test-only function set (`SUM`, `IF`,
`INDIRECT`, `NOW`, `AI`) on `FnRegistry`. Production `FnRegistry::new()` has
none of those; unknown names still evaluate to `#NAME?`.

## Expected values

| Token | Meaning |
|---|---|
| `TRUE` / `FALSE` | Booleans |
| `#DIV/0!`, `#NAME?`, `#N/A`, `#SPILL!`, … | Excel error display strings |
| `{1,2;3,4}` | Array (row-major; `;` separates rows) |
| `"text"` | Text (quotes stripped) |
| empty / `(empty)` | Empty cell |
| other | Parsed as `f64` (integers may be written without a decimal) |
