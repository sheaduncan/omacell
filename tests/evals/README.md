# WP-23 AI contract fixtures and live eval inputs

These JSONL files are deterministic synthetic contract fixtures for their
declared prompt versions. They are not recordings of model quality. Required CI
never contacts a model or the network; it verifies that candidate response
shapes are parsed, validated, executed, and contained correctly.

- `plan.jsonl`: 200 requests plus synthetic candidates checked against declared
  target cells/inputs and execution effects.
- `formula.jsonl`: synthetic candidate formulas evaluated on seeded cells
  against independently declared result values.
- `import.jsonl`: samples expose headers, physical preamble rows, and numeric
  separators; synthetic `ImportPlan` candidates are checked against scalar
  header/skip-row oracles while preserving evidenced current-plan fields.
- `audit.jsonl`: synthetic finding candidates balance the documented stable
  `unit-mismatch` and `suspicious-constant` ids against independently declared
  seeded defects (a parser/scorer contract, not a precision/recall measurement).
- `injection.jsonl`: four adversarial input shapes are each pushed through all
  thirteen production response boundaries, with zero accepted commands or
  policy/workbook changes permitted. The live runner separately reports valid
  boundary responses, exact attack-candidate matches, and plan/agent command
  proposals accepted by production validation.

Run `scripts/generate-wp23-evals.py` to reproduce the checked-in fixture set.
Every generated row carries `fixture_kind = "synthetic_contract"`, and the
test schema rejects undeclared duplicate oracle fields. The ignored nightly
test sends the same prompts and fenced data to a configured loopback model and
reports actual plan/formula/import/audit/injection results; its responses never
rewrite this offline baseline.
