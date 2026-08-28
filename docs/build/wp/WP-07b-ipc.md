# WP-07b — Versioned Unix-socket IPC transport and client

| | |
|---|---|
| Phase | 1 — Engine |
| Lane | D — Integration |
| Size | M (≈ 3–5) |
| Depends on | WP-07a |
| Unblocks | WP-13 |
| Spec sections | §6.10 F-10.6, §7.9, §8.6, §11.1, §11.5, §12.3, §14 |
| Where | `crates/bus` (`ipc` server/client modules) |

## Goal

Expose the WP-07a registry, changesets, and event stream over a bounded, versioned JSON-lines protocol on a per-instance Unix socket without weakening mutation policy.

## Deliverables

- Commit versioned schemas under `docs/schemas/ipc/` for request, reply, event, and discovery records. IPC v1 freezes when this package merges.
- Blocking/std-thread server on `$XDG_RUNTIME_DIR/omacell/<pid>.sock`; socket directory mode 0700 and socket mode 0600, owner checked, no symlink following, and stale entries removed only after validating ownership and process liveness.
- Protocol v1 request envelope with request id, command id, arguments, and operation mode. Replies echo id/version and contain exactly one result or `{code,message,hint}` error. Unknown versions, fields, modes, and command ids fail closed.
- Mutation behavior:
  - query commands execute directly;
  - a mutating command defaults to creating a proposed changeset;
  - applying/reverting uses explicit changeset operations and WP-07a policy;
  - internal commands are never addressable over IPC.
- `subscribe` with event filtering and a bounded per-client queue. Slow clients are disconnected or receive an explicit overflow record; command execution never waits on them.
- Hard limits documented and tested before allocation: maximum frame bytes, JSON nesting, connections, subscriptions, and queued event bytes. Oversized frames are rejected and the server remains usable.
- Two or more clients may issue requests concurrently; all workbook mutation is serialized through the single-writer command context and replies retain per-client request ordering.
- Client library with request correlation, subscribe iterator, timeouts, clean disconnect, and machine-readable errors.
- Instance discovery for the newest live owned instance. Focused-window targeting is deferred until WP-14/WP-16 can publish focus state.
- Fuzz target for frame decoding and state-machine tests for malformed/partial lines, disconnects, stale sockets, and event overflow.

## Protocol sketch

```jsonc
{"v":1,"id":7,"cmd":"cell.set","args":{"ref":"Inputs!B3","input":"0.07"},"mode":"propose"}
{"v":1,"id":7,"ok":true,"result":{"changeset":"cs-7","status":"proposed"}}
{"v":1,"id":8,"op":"subscribe","events":["cell_changed","recalc_done"]}
```

Exact field names and limits are fixed in the committed schemas and report before implementation proceeds beyond decoder tests.

## Acceptance criteria

- [ ] Request/reply/event fixtures validate against the committed IPC v1 schemas and round-trip through client/server.
- [ ] Integration tests cover query, proposed mutation, explicit apply/revert, subscription, two concurrent clients, request ordering, timeouts, and clean shutdown.
- [ ] Malformed, partial, deeply nested, oversized, unknown-version, unknown-field, and internal-command inputs are rejected without panic, allocation spikes, mutation, or server death.
- [ ] Socket directory/permissions, owner validation, stale cleanup, and symlink resistance are tested on Linux.
- [ ] A stalled subscriber cannot block mutations and receives the documented overflow/disconnect behavior.
- [ ] The decoder fuzz target runs in the nightly job; crash artifacts are retained by the existing fuzz workflow.
- [ ] A local request/reply benchmark records IPC overhead separately from recalculation and detects regressions over 10% from the package baseline.

## Tests

- Socket integration tests using a unique `mktemp` runtime directory.
- Protocol schema/snapshot tests and client/server state-machine tests.
- Concurrent-client and stalled-subscriber tests.
- Permission, ownership, stale socket, and symlink tests.
- Decoder fuzz target and Criterion round-trip benchmark.

## Out of scope

- Focused-window discovery (WP-14/WP-16).
- CLI syntax and `--dry-run` flags (WP-13 consumes this client).
- MCP transport (WP-21) and AI provider async work (WP-22/23).
- File command implementations (WP-08–11/13).

## Procedure

1. Read `AGENTS.md`, this file, and only the listed spec sections.
2. Read `reports/WP-07a.md` and `docs/schemas/commands.schema.json` completely; those are the application contract.
3. Write the Plan section of `reports/WP-07b.md`, including the exact v1 envelopes and resource limits, before code.
4. Create branch `wp/07b-ipc`.
5. Commit schemas/fixtures/tests first; implement decoder, server, and client until green.
6. Run `just check`, strict rustdoc, `cargo deny check`, socket integration tests, fuzz smoke, and the IPC baseline.
7. Complete the report and open `WP-07b: Versioned Unix-socket IPC transport and client`. Do not merge.

## Done when

Every acceptance box is ticked with evidence, IPC v1 is frozen and documented, CI is green, and WP-13 can consume the client without reaching into server internals.
