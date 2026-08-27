# WP-22 — AI provider layer, privacy and redaction, workbook card, audit log

| | |
|---|---|
| Phase | 5 — Scripting, agents, AI |
| Lane | D — Integration (bus, CLI, Lua, MCP, AI, release) |
| Size | L (≈ 6–10) |
| Depends on | WP-12, WP-07, WP-19 |
| Unblocks | WP-23 |
| Spec sections | §8.1, §8.2, §8.7, §8.8 (providers, routing), §11.2 ADR-005, §12.3 |
| Where | `crates/ai` (modules `provider`, `openai_compat`, `anthropic`, `secrets`, `card`, `redact`, `policy`, `audit`, `record`) |

## Goal

Talk to any model through two wire protocols, never leak more than the privacy level allows, and describe workbooks to models compactly — all testable offline.

## Deliverables

- `Provider` trait (async, `tokio`): chat with structured-output (JSON schema) and tool-calling, streaming, cancellation, timeouts; `openai_compatible` (chat completions; covers Ollama, LM Studio, llama.cpp, vLLM, OpenRouter, cloud vendors) and `anthropic` (Messages API) implementations over `reqwest` + `rustls`; `local` detection for loopback endpoints.
- Secrets: `secret_env` and `secret_cmd` resolution only; a test greps the config dir and logs for known secret shapes after every operation.
- Model slots (`fast|default|strong|agent|vision`) and routing; budgets and rate limits (`[ai.functions]`), request accounting for `omacell ai usage --json`.
- Workbook card builder: levels `summary|columns|sample|full` per §8.2 with a token-budget estimator, selection-aware focus using the dependency graph, stable JSON schema (`docs/schemas/card.schema.json`); `omacell ai card` CLI.
- Privacy policy enforcement in the payload builder (level, per-workbook override stored in the custom part, loopback defaults); redaction: `ai.redact` marks, pattern detectors (email, phone, card-like, national-ID shapes, IBAN) producing suggestions, placeholder rendering `[REDACTED:kind]`; applies to cards, cell inputs, and images.
- Audit log `~/.local/state/omacell/ai/log.jsonl` (task, provider, model, sizes, hashes, latency; content only with `log_content = true`); `omacell ai log`.
- Record/replay harness for tests (`tests/fixtures/ai/*.json`): providers run against recorded exchanges in CI; recording mode for humans with a real endpoint.
- `omacell ai setup`: detect Ollama/LM Studio, write only `config.toml`, print the endpoint that will be used, never store keys; status-segment data source (provider, level, session sends).

## Interface sketch

```rust
#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, AiError>; // supports json_schema + tools + stream
    fn is_local(&self) -> bool;
    fn name(&self) -> &str;
}
```

## Implementation notes

- No network in CI: every provider test runs on recorded fixtures; the recorder is a dev tool.
- The payload builder is the single choke point for privacy — nothing else may serialize workbook content for a model.

## Acceptance criteria

- [ ] Recorded-fixture tests for both protocols: structured output, tool calls, streaming, errors, timeouts, cancellation.
- [ ] Privacy tests: `schema` level payloads contain no cell values; redaction applied on every payload path; per-workbook override respected; loopback defaults.
- [ ] Secret-leak test passes; card golden tests at every level; budget/rate-limit tests.
- [ ] `omacell ai setup` in a temp `$HOME` with fake local servers writes the expected config and nothing else.

## Tests

- Fixture-driven provider tests; privacy/redaction tests; golden card tests; leak test.

## Procedure

1. Read `AGENTS.md`, this file, and only the spec sections listed above.
2. Read `reports/<dep>.md` for every package in *Depends on* — their *Interfaces exposed* sections are your inputs.
3. Write the *Plan* section of `reports/WP-22.md` before writing code.
4. Create branch `wp/22-ai-providers-privacy-card`.
5. Write the corpora/fixtures/tests named above first; implement until they pass; run `just check`.
6. Complete the report (template: `docs/build/templates/wp-report.md`), tick the acceptance boxes you can prove, and open a PR titled `WP-22: AI provider layer, privacy and redaction, workbook card, audit log`. Do not merge.

## Done when

Every acceptance box is ticked with evidence in the report, CI is green, the report is complete, and no new `TODO(` lacks a `WP-` reference.
