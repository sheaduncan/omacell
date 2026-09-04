# AI and privacy

AI is off until configured. Omacell has no telemetry and makes no implicit
network request. Local OpenAI-compatible endpoints can be discovered with
`omacell ai setup`; cloud providers require an explicit provider, model, and
credential configuration.

Before provider dispatch, the privacy layer applies the configured sharing
policy and verifies that the private local audit log is writable and has room
for the request record. The completed record is appended after the response;
an append failure at that point is reported locally and never causes the
successful request to be sent again. Workbook content is not automatically
attached wholesale. Agent edits are proposals: mutating commands cross the
changeset review boundary and must be reviewed unless the user has deliberately
enabled bounded autopilot.

Worksheet AI functions pass evaluated arguments through that same boundary.
At `schema` level, workbook-data arguments become typed shapes while the
function's explicit instruction arguments remain available to perform the
requested task. Pattern detectors and accepted `ai.redact` marks apply before
the batch is fenced as data.

Credentials belong in the configured secret source, not a workbook, prompt,
Lua script, repository file, or command-line argument. Provider prompts and
responses may still be retained by the chosen provider; consult that service's
policy. Use a local endpoint for material that must not leave the machine.
AI provider requests connect only to the configured endpoint: Omacell ignores
ambient system-proxy environment variables and does not follow HTTP redirects.
Configure a gateway as the provider endpoint when one is deliberately required.

Embedded workbook Lua cannot register AI tasks or hooks, invoke prompts, alter
keys, or gain new command capabilities merely because a command was added.
See the [Lua API](lua-api.md) for both runtime profiles.
