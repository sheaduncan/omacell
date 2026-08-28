# Corpus — functions

TSV grammar for `tests/corpus/functions/<NAME>.tsv`:

```
# comment
formula<TAB>expected<TAB>note[<TAB>locale][<TAB>date_system]
```

- `formula` includes the leading `=`.
- `expected` is the committed display text (`format_cell` for scalars; `{a,b;c,d}` for unblocked spills).
- `note` cites the Excel behaviour the row encodes.
- `locale` (optional) is a BCP-47 tag (`en-US`, `en-GB`, `de-DE`) applied via `RecalcEngine::set_locale`.
- `date_system` (optional) is `1900` or `1904` and is written to `WorkbookSettings.date_system`.

The shared runner is `omacell_fn::run_corpus_file`. It rejects malformed rows, registers `register_all()`, and uses an injected clock (`45000.5`) and nonce. `scripts/lo-crosscheck.py` evaluates the same rows through a temporary headless LibreOffice workbook; notes containing `known difference` must correspond to an entry in `docs/compat/known-differences.md`.

WP-05a fills math/stat/logical/information/criteria-aggregation corpora (≥10 cited rows per function). WP-05b fills text/date corpora. `SEQUENCE` remains a WP-05c probe.

TODO(WP-05c): lookup/array/financial function corpus.
