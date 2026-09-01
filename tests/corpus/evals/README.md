# Corpus — evals

Recorded AI-provider HTTP exchanges live in `tests/fixtures/ai/` (WP-22; no
network in CI). Replay them through `omacell_ai::ReplayTransport`.

The committed WP-23 plan, formula, import, audit, and injection corpora live in
`tests/evals/`; `crates/ai/tests/evals.rs` scores them entirely offline.
Recorded HTTP for cell batches uses in-test `Transport` doubles. The nightly
workflow separately scores the same corpus against a loopback local model.
