# Corpus — functions

TSV grammar for `tests/corpus/functions/<NAME>.tsv`:

```
# comment
formula<TAB>expected<TAB>note
```

- `formula` includes the leading `=`.
- `expected` is the committed display text (`format_cell`).
- `note` cites the Excel behaviour the row encodes.

The shared runner is `omacell_fn::run_corpus_file` / `assert_corpus_file`. It registers WP-05F probes (`ABS`, `SUM`, `IF`, `NOW`, `RAND`, `SEQUENCE`) with an injected clock and nonce.

TODO(WP-05a): math/stat/logic function corpus.
TODO(WP-05b): text/date function corpus.
TODO(WP-05c): lookup/array/financial function corpus.
