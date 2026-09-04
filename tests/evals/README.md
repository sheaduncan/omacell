# WP-23 AI contract fixtures and live eval inputs

These JSONL files are deterministic synthetic contract fixtures for prompt
version 1. They are not recordings of model quality. Required CI never contacts
a model or the network; it verifies that candidate response shapes are parsed,
validated, executed, and contained correctly.

- `plan.jsonl`: 200 requests plus synthetic candidates checked against declared
  target cells/inputs and execution effects.
- `formula.jsonl`: synthetic candidate formulas evaluated on seeded cells
  against independently declared result values.
- `import.jsonl`: synthetic `ImportPlan` candidates checked for bounded valid
  overlays derived from each input sample.
- `audit.jsonl`: synthetic finding candidates parsed against declared seeded
  defect ids (a parser/scorer contract, not a precision/recall measurement).
- `injection.jsonl`: adversarial synthetic candidates pushed through every
  response boundary, with zero commands or policy/workbook changes permitted.

Run `scripts/generate-wp23-evals.py` to reproduce the checked-in fixture set.
Every generated row carries `fixture_kind = "synthetic_contract"`, and the
test schema rejects undeclared duplicate oracle fields. The ignored nightly
test sends the same prompts and fenced data to a configured loopback model and
reports actual plan/formula/import/audit/injection results; its responses never
rewrite this offline baseline.
