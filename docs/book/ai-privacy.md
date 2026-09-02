# AI and privacy

AI is off until configured. Omacell has no telemetry and makes no implicit
network request. Local OpenAI-compatible endpoints can be discovered with
`omacell ai setup`; cloud providers require an explicit provider, model, and
credential configuration.

Before provider dispatch, the privacy layer applies the configured sharing
policy and records the request class in the local audit log. Workbook content
is not automatically attached wholesale. Agent edits are proposals: mutating
commands cross the changeset review boundary and must be reviewed unless the
user has deliberately enabled bounded autopilot.

Credentials belong in the configured secret source, not a workbook, prompt,
Lua script, repository file, or command-line argument. Provider prompts and
responses may still be retained by the chosen provider; consult that service's
policy. Use a local endpoint for material that must not leave the machine.

Embedded workbook Lua cannot register AI tasks or hooks, invoke prompts, alter
keys, or gain new command capabilities merely because a command was added.
See the [Lua API](lua-api.md) for both runtime profiles.
