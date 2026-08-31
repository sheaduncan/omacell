# Corpus — evals

Recorded AI-provider HTTP exchanges live in `tests/fixtures/ai/` (WP-22; no
network in CI). Replay them through `omacell_ai::ReplayTransport`.

Plan evals and the injection suite live in `crates/ai/tests/features.rs` (200 NL→plan rows + forbidden-command payloads). Recorded HTTP for cell batches uses in-test `Transport` doubles.
