# AI provider fixtures

Recorded OpenAI-compatible and Anthropic HTTP exchanges. Tests replay these
through `omacell_ai::ReplayTransport`. No network.

For an explicit human recording session, wrap `ReqwestTransport` in
`RecordingTransport`. It writes private, replay-compatible files without HTTP
headers. Request and response bodies can contain workbook data, so inspect and
rename a recording before moving it into this directory.

The committed set also contains hand-authored malformed-response fixtures.
They prove both adapters fail closed on invalid provider tool calls.
