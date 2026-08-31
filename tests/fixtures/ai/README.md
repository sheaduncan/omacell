# AI provider fixtures

Recorded OpenAI-compatible and Anthropic HTTP exchanges. Tests replay these
through `omacell_ai::ReplayTransport`. No network.

To record a new exchange against a real endpoint, capture the JSON POST path
and body plus the response (or SSE JSON payloads) into a file named
`<protocol>_<case>.json` matching `RecordedExchange` in `crates/ai/src/http.rs`.
