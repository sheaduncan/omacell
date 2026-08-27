# ADR-005 — AI providers are wire protocols, not SDKs

| | |
|---|---|
| Status | **Decided** |
| Date | 2026-08-27 |
| Spec | §8.1, §11.2 |
| Plan default (D8) | OpenAI-compatible and Anthropic Messages; no vendor SDKs |

## Decision

Omacell speaks two protocols — OpenAI-compatible chat completions (Ollama,
LM Studio, llama.cpp, vLLM, OpenRouter, most cloud vendors) and the
Anthropic Messages API — with structured output and tool calling on both.
No per-vendor SDKs. A provider is a TOML block. Async is confined to
`omacell-ai` and MCP via `tokio`.
