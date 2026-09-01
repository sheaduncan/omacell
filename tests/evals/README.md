# WP-23 offline AI evals

These JSONL files are committed, deterministic response recordings for prompt
version 1. Required CI never contacts a model or the network.

- `plan.jsonl`: 200 natural-language requests, recorded structured replies,
  exact expected commands, and execution-effect equivalence.
- `formula.jsonl`: generated formulas evaluated on seeded fixture cells.
- `import.jsonl`: `ImportPlan` overlays compared with expected plans.
- `audit.jsonl`: AI-finding precision/recall against seeded unit defects.
- `injection.jsonl`: instruction-shaped workbook data passed through every AI
  task family, with zero model commands and zero policy changes permitted.

Run `scripts/generate-wp23-evals.py` to reproduce the checked-in fixture set.
The nightly workflow runs the same scoring contract against a local small
model; its responses never rewrite this offline baseline.
