# Report — WP-07b: Versioned Unix-socket IPC transport and client

## Plan (written before coding)

- Files/modules to create:
  - `docs/schemas/ipc/{request,reply,event,discovery}.schema.json` — IPC v1 freeze.
  - `tests/fixtures/ipc/` — valid and invalid JSON-line fixtures.
  - `crates/bus/src/ipc/{mod,protocol,server,client,discover}.rs` — decoder, bind/accept, client, discovery.
  - `crates/bus/src/error.rs` — additive `ipc.*` codes.
  - `crates/bus/tests/{ipc_protocol,ipc_server}.rs`.
  - `crates/bus/benches/ipc_roundtrip.rs`.
  - `fuzz/fuzz_targets/ipc_frame.rs` (nightly job already iterates `cargo fuzz list`).
  - `docs/contracts.md` — IPC v1 freeze note. No WP-01 type changes.
- Interfaces to expose (types, commands, schemas, CLI):
  - `omacell_bus::ipc::{IpcServer, IpcHandle, IpcClient, Request, Reply, ServerRecord, discover_newest, limits}`.
  - No CLI (`omacell ipc` is WP-13). No MCP. No focused-window targeting.
  - Origin on the wire is always `Origin::Ipc`. Internal command ids are never dispatched.
- Tests and corpora to write first:
  - Schema validation + fixture round-trip for request/reply/event/discovery.
  - Decoder: malformed, partial, nested, oversized, unknown version/field/mode/op, both `cmd` and `op`.
  - Integration: query, propose mutation, apply/revert, subscribe, two clients, request ordering, timeout, shutdown.
  - Linux: dir 0700 / sock 0600, owner check, stale cleanup, symlink refusal.
  - Stalled subscriber: mutations proceed; client gets overflow then disconnect.
  - Fuzz target + criterion ping/propose overhead (no recalc in the timed path).
- Items the package says to "decide and document" and the decision taken:
  - **Envelope version:** `v: 1`. Integer, required, fail closed otherwise.
  - **Command vs control:** a request has either `cmd` (registry command) or `op` (control). Never both, never neither.
  - **`mode`:** `propose` | `execute` | `dry_run`. Omitted:
    - query command → execute;
    - mutating + changeset-eligible → propose;
    - mutating + not eligible (`calc.recalc`, `edit.undo`, `edit.redo`) → execute (Ipc is trusted in-process; these cannot be proposed).
    - `mode: execute` on a changeset-eligible mutating command is **rejected** (`ipc.mode`) so the socket cannot bypass review.
  - **Control `op`:** `subscribe`, `unsubscribe`, `changeset.apply`, `changeset.revert`, `changeset.list`, `changeset.get`, `ping`.
  - **Event filter names:** frozen Event `type` strings (`cell_changed`, `recalc_done`, …). Empty `events` on subscribe means all types.
  - **Overflow:** per-client queue 64 events / 256 KiB. On overflow the server writes an overflow record and disconnects. Command dispatch never waits on a client write beyond a short socket timeout.
  - **Limits:** see table below. Checked before allocating the parsed value (frame size and nesting) or before accept (connection count).
  - **Discovery:** `$XDG_RUNTIME_DIR/omacell/<pid>.sock` plus sibling `<pid>.instance`. Newest live owned instance = greatest `started_unix_ms` among sockets whose owner is the current euid and whose pid is alive (`/proc/<pid>`). Tests inject a temp dir.
  - **Threading:** std threads only (no tokio). One accept thread, one thread per client, `Arc<Mutex<Bus>>` single-writer.
- Open questions at planning time:
  1. Should `edit.undo` / `edit.redo` be forbidden on the socket? Plan: allowed as direct execute (not eligible for propose). WP-13 can hide them.
  2. Focused-window targeting waits for WP-14/WP-16.

### IPC v1 envelopes (frozen)

Command request:

```json
{"v":1,"id":7,"cmd":"cell.set","args":{"ref":"A1","input":"0.07"},"mode":"propose"}
```

Control request:

```json
{"v":1,"id":8,"op":"subscribe","events":["cell_changed","recalc_done"]}
{"v":1,"id":9,"op":"changeset.apply","changeset":"cs-1"}
{"v":1,"id":10,"op":"ping"}
```

Reply (exactly one of `result` or `error`):

```json
{"v":1,"id":7,"ok":true,"result":{"changeset":"cs-1","status":"proposed"}}
{"v":1,"id":7,"ok":false,"error":{"code":"command.unknown","message":"...","hint":"..."}}
```

Event / overflow records (unsolicited; no `id`):

```json
{"v":1,"kind":"event","event":{"type":"cell_changed","sheet":0,"row":0,"col":0}}
{"v":1,"kind":"overflow","dropped":4}
```

Discovery record (`<pid>.instance`):

```json
{"v":1,"pid":1234,"socket":"1234.sock","started_unix_ms":0}
```

### Resource limits

| Limit | Value |
|---|---|
| `MAX_FRAME_BYTES` | 16,777,216 hard ceiling and default |
| `[ipc].max_frame_bytes` | 1,048,576–16,777,216; restart to apply |
| `MAX_JSON_DEPTH` | 32 |
| `MAX_CONNECTIONS` | 32 |
| `MAX_EVENT_FILTERS` | 16 |
| `MAX_EVENT_QUEUE` | 64 |
| `MAX_EVENT_QUEUE_BYTES` | 262,144 |
| Socket directory mode | 0700 |
| Socket mode | 0600 |
| Default request timeout (client) | 5s |

### 2026-09-04 IPC lifecycle follow-up plan (written before coding)

- Reproduce the remaining WP-07b findings from
  `reports/review-2026-09-02.md` before implementation: a leftover socket for
  the current pid cannot be removed because `pid_is_alive` is true, apply does
  not record the workbook generation it was proposed against, changeset ids
  restart at `cs-1` whenever a store is constructed, `FilterCriteriaArg`
  accepts unknown fields, and `script.*` / `macro.*` default to direct execute
  on the socket.
- Replace pid-liveness as the bind/discovery oracle with a connect probe: an
  owned socket without a listener is stale even when that pid is alive (crash
  plus pid reuse). Refuse to treat a leftover socket as a live instance, and
  remove owned companions (`.sock`, `.instance`, `.focus`) only after that
  probe. Keep symlink refusal.
- Record a private live-generation on each proposal. Any successful live
  mutating command, apply, or revert advances the generation; apply of a
  proposal whose generation no longer matches fails closed with a dedicated
  `changeset.base` code before dispatch. Do not add fields to the frozen
  `Changeset` type or IPC v1 envelopes.
- Assign collision-resistant opaque ids (`cs-{session}-{seq}`) from a
  process-wide counter so opening a workbook, which reconstructs the store,
  cannot reissue `cs-1`. Keep `ChangesetId` an opaque non-empty string.
- Add `#[serde(deny_unknown_fields)]` to `FilterCriteriaArg`. Reject
  `script.*` and `macro.*` execute over IPC; keep `edit.undo` / `edit.redo` /
  `calc.recalc` as the documented same-user session-private execute set.
- Tests first: leftover current-pid bind and discovery, intervening-mutation
  apply, unique ids across store reset, unknown filter fields, script/macro
  socket execute, and undo execute remaining allowed. No IPC schema change.

### 2026-09-04 proposal-base escape-hatch follow-up plan (written before coding)

- Reproduce the remaining base-generation hole: `live_generation` advances only
  after mutating commands, so `workbook_mut`, `engine_mut`, `registry_mut`, and
  `recalc_after_registry_change` can change workbook or formula semantics while
  a reviewed proposal still applies.
- Fail closed: every public mutable escape hatch and registry-driven recalc
  must bump the same generation used by `changeset.base`. Do not change the
  frozen `Changeset` type.
- Rename `apply_rechecks_retained_size_before_live_mutation` to a
  base-generation precedence test. Add a store-level regression that calls
  `ensure_applied_fits` with an oversized inverse so the retained-size check is
  covered without an intervening live command.
- Tests first: direct `workbook_mut` after propose, function/engine refresh
  after propose, and `ensure_applied_fits` limit rejection.

## What was built

Versioned JSON-lines IPC on a per-instance Unix socket, wrapping the WP-07a bus without weakening mutation policy.

Key files:

- [`docs/schemas/ipc/`](../docs/schemas/ipc/) — request, reply, event/overflow, discovery (v1 freeze)
- [`crates/bus/src/ipc/protocol.rs`](../crates/bus/src/ipc/protocol.rs) — fail-closed decoder, frame cap, nesting cap
- [`crates/bus/src/ipc/server.rs`](../crates/bus/src/ipc/server.rs) — std-thread accept/client loops, `Arc<Mutex<Bus>>`
- [`crates/bus/src/ipc/client.rs`](../crates/bus/src/ipc/client.rs) — correlated requests, subscribe, timeouts
- [`crates/bus/src/ipc/discover.rs`](../crates/bus/src/ipc/discover.rs) — 0700/0600, owner, stale, symlink refusal
- Tests: [`ipc_protocol.rs`](../crates/bus/tests/ipc_protocol.rs), [`ipc_server.rs`](../crates/bus/tests/ipc_server.rs)
- Fuzz: [`fuzz/fuzz_targets/ipc_frame.rs`](../fuzz/fuzz_targets/ipc_frame.rs) (picked up by nightly `cargo fuzz list`)
- Bench: [`crates/bus/benches/ipc_roundtrip.rs`](../crates/bus/benches/ipc_roundtrip.rs)

Review hardening added before merge:

- Shutdown now stops and joins active client threads, removes every subscription on all exit paths, and reaps completed thread handles during normal operation.
- Subscription filters and the 256 KiB budget are enforced before events enter a client queue; unrelated events cannot force a filtered subscriber to overflow.
- Client-side unsolicited events remain FIFO. Reply/event envelopes reject unknown fields and unsupported versions, while a valid JSON `null` result remains distinguishable from a missing result.
- Discovery treats metadata as untrusted: socket paths are reconstructed from the validated pid, instance files are size/type/owner checked, and failed startup removes a newly bound socket without following or deleting a pre-existing symlink.
- The active frame limit includes the newline without transiently buffering an oversized frame, and control-op schemas exactly match the decoder's op-specific fields.
- The hard/default limit is now 16 MiB and `[ipc].max_frame_bytes` can lower it to 1–16 MiB. Server decode/encode, clients, MCP, and the Python stdio bridge use the same validated limit within each process.
- The shared bus/runner dispatcher rejects `edit.repeat` in IPC execute mode.
  Repeat expands session-private state into an arbitrary prior mutation, so IPC
  callers must submit that original command as a reviewable proposal instead.
  Socket regressions cover omitted and explicit execute modes on both server
  implementations; trusted in-process user and script repeat behavior is unchanged.

The post-WP-16 integration adds focused-instance routing without changing IPC
v1. `IpcHandle::set_focused` maintains a zero-byte, owned mode-0600
`<pid>.focus` companion marker. GUI native focus and TUI terminal focus events
publish it; default discovery prefers the newest valid focused live instance
and falls back to the original newest-live rule. Startup and shutdown clear
recycled markers, and marker symlinks are refused without touching their target.

2026-09-04 IPC lifecycle follow-up:

- Stale-socket removal uses a connect probe instead of pid liveness, so a
  leftover `{pid}.sock` after crash-plus-pid-reuse is not treated as live and
  can be replaced on bind. Owned `.instance` / `.focus` companions are removed
  with the stale socket. Tests:
  `leftover_socket_for_a_live_pid_without_a_listener_is_stale`,
  `stale_socket_for_dead_pid_is_removed`.
- Propose records a private live-generation; any successful live mutating
  command, apply, or revert advances it. Apply of a stale proposal fails with
  `changeset.base` before dispatch. Tests:
  `apply_rejects_a_proposal_after_an_intervening_live_mutation`,
  `apply_succeeds_when_the_workbook_generation_is_unchanged`,
  `applying_one_proposal_invalidates_a_sibling_proposed_at_the_same_base`.
- Changeset ids are `cs-{session:016x}-{seq}` from a process-wide counter, so
  reconstructing the store cannot reissue `cs-1`. Test:
  `store_ids_do_not_restart_at_cs_1_across_new_stores`.
- `FilterCriteriaArg` now `deny_unknown_fields`. `script.*` and `macro.*`
  cannot execute over IPC; `edit.undo` / `edit.redo` / `calc.recalc` remain
  the documented same-user session-private execute set. Tests:
  `filter_criteria_arg_rejects_unknown_fields`,
  `script_and_macro_commands_cannot_execute_over_ipc`.

Proposal-base escape-hatch follow-up: `workbook_mut`, `engine_mut`,
`registry_mut`, and `recalc_after_registry_change` bump the same live
generation as mutating commands, so a reviewed proposal cannot apply after
out-of-band workbook or function-registry changes. Tests:
`apply_rejects_a_proposal_after_direct_workbook_mutation`,
`apply_rejects_a_proposal_after_function_registry_refresh`,
`registry_refresh_invalidates_outstanding_proposal_bases`,
`store_rejects_an_oversized_applied_inverse`. The former retained-size
apply test is now
`apply_base_generation_check_precedes_retained_size_recheck`.

## Interfaces exposed (for dependents)

| Item | Where |
|---|---|
| `ipc::serve`, `ipc::serve_with_limits`, `IpcHandle` | bind `{runtime_dir}/{pid}.sock`; configured variants enforce `IpcLimits`; `set_focused` publishes frontend focus |
| `ipc::IpcClient` | `connect` / `connect_with_limits` / `connect_focused` / `connect_newest` / `connect_default` / `connect_default_with_limits` / `command` / `control` / `apply` / `revert` / `subscribe` / `poll_record` |
| `ipc::{Request, Reply, ServerRecord, Discovery, Mode, ControlOp}` | v1 envelopes |
| `ipc::{decode_request, decode_request_bytes, IpcLimits, MAX_*}` | decoder + hard/runtime limits |
| `ipc::{discover_focused, discover_default, discover_newest, default_runtime_dir}` | focused/default/newest live owned instance selection |
| Error codes | `ipc.version`, `ipc.frame`, `ipc.protocol`, `ipc.mode`, `ipc.limit`, `ipc.socket`, `ipc.timeout`, `ipc.disconnected`, `ipc.overflow`, `changeset.base` |
| Schemas | `docs/schemas/ipc/*.schema.json` |

WP-13 should use `IpcClient`; it must not reach into server internals. Origin on the wire is always `Origin::Ipc`.

## Deviations from the spec or the package (with reasons)

- **Spec F-10.6 sketch `{id, cmd, args}`** is extended with `v`, optional `mode`, and control `op` as planned. Replies still echo `id` and carry exactly one of `result`/`error`.
- **Event filter names** use frozen `Event` tags (`cell_changed`) rather than spec prose `cell.changed`.
- **`edit.undo` / `edit.redo` / `calc.recalc`** execute directly (not changeset-eligible). Eligible mutating commands cannot use `mode: execute`.
- **Focused-window discovery was completed after WP-14/WP-16.** The frozen discovery JSON stays unchanged; a private companion marker carries ephemeral focus state. `discover_newest` retains its exact original behavior for explicit callers.
- **Stale-socket liveness is a connect probe, not `/proc/<pid>`.** A leftover socket whose pid has been reused by a live process is stale if nothing is listening. This matches the package's "stale entries removed only after validating ownership and process liveness" rule under pid reuse.
- **`script.*` / `macro.*` cannot execute on the socket.** Same-user CLI still needs undo/redo/recalc and file lifecycle over IPC; those stay direct-execute. MCP origin cleanup for open/save/export remains WP-21.

## Measurements

Host: local Linux.

- `cargo test -p omacell-bus` — protocol 8, server 15, one event-queue unit test, plus existing bus tests, all pass.
- `just check` — green.
- `RUSTDOCFLAGS="-D warnings" cargo doc -p omacell-bus --no-deps` — pass.
- `cargo deny check` — advisories/bans/licenses/sources ok.
- `ASAN_OPTIONS=detect_leaks=0 cargo +nightly fuzz run ipc_frame -- -runs=10000` — 10,000 executions, no crash (`detect_leaks=0` is required because this review environment blocks LeakSanitizer's ptrace use).
- `cargo bench -p omacell-bus --bench ipc_roundtrip -- --noplot` after review hardening:
  - `ipc_roundtrip/ping` — 10.5 µs (10.3–10.7)
  - `ipc_roundtrip/cell_set_propose` — 51.5 µs (49.5–54.0), no workbook recalc in the timed path.
- Focus integration: the bus test proves a focused older instance beats a
  synthetically newer live instance, startup/shutdown cleanup, and symlink
  refusal; the black-box CLI test reaches that focused instance by default;
  the GUI lifecycle test publishes and clears native focus.
- Frame-limit integration: `just check` is green, including strict workspace
  Clippy, all workspace tests, and rustdoc. Focused suites pass with 10 protocol
  tests, 21 server/client tests, 15 configuration-layering tests, and 3 CLI
  scripting tests. Coverage includes >1 MiB default requests, a lowered 1 MiB
  setting, client and server rejection, and oversized replies converted into a
  correlated `ipc.frame` response.
- Repeat-boundary follow-up: `cargo test -p omacell-bus` passes, including 22
  server/client tests; the 3-test CLI scripting bridge suite, strict workspace
  Clippy, and workspace rustdoc also pass.
- IPC lifecycle follow-up (2026-09-04): `cargo test -p omacell-bus` passes
  (24 server/client tests, 20 changeset tests, 10 data-tool tests).
  `cargo clippy -p omacell-bus --all-targets -- -D warnings` passes.
  `RUSTDOCFLAGS="-D warnings" cargo doc -p omacell-bus --no-deps` passes.
  CLI `changeset` and `ipc_focus` tests pass.
  `CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=/home/duncan/.cache/omacell/target just check`
  is green (fmt, workspace clippy `-D warnings`, workspace tests, rustdoc).

No new product-graph crates.io dependencies. `criterion` is workspace-dev (pre-approved). `libfuzzer-sys` remains fuzz-workspace only.

## Open questions / decisions needed

1. **Resolved:** do not hide undo/redo from same-user IPC or script origins;
   agent origins remain changeset-only.
2. **Resolved:** keep 32 connections and raise the hard/default frame limit to
   16 MiB, with `[ipc].max_frame_bytes` able to lower it to 1–16 MiB. Large
   operations that exceed the active limit must be split into multiple range
   commands.

## RFC (only if a frozen contract changed)

This follow-up does not change IPC v1 envelopes, discovery JSON, or the frozen
`Changeset` struct. `ChangesetId` remains an opaque non-empty string; only the
bus assignment policy changes. `changeset.base` is an additive bus error code.
`FilterCriteriaArg` unknown-field rejection is the registry's existing
`deny_unknown_fields` contract, applied to the one public arg enum that lacked
it.

The owner-provided WP-07b.2 decision and instruction to complete the pre-WP-28
integration queue approve raising the frozen frame limit from 1 MiB to a 16 MiB
hard/default ceiling. The additive `[ipc].max_frame_bytes` key may lower the
active limit to 1–16 MiB and is read at process startup; changing it requires a
restart. The 32-connection cap, JSON-lines envelopes, version 1, nesting cap,
mutation policy, and error-code taxonomy do not change. Existing peers that
still enforce 1 MiB can reject larger frames, so the failure hint tells callers
to split large ranges into multiple commands. Oversized frames remain
`ipc.frame`; `ipc.limit` continues to identify invalid limit configuration and
other resource-limit failures. With 32 simultaneously incomplete maximum-size
frames, receive buffers are bounded at roughly 512 MiB plus connection and JSON
overhead; operators can retain the old 1 MiB limit where that tradeoff is
preferable.

## Checklist

- [x] `just check` green on a clean clone
- [x] Every acceptance criterion ticked with evidence
- [x] Docs warning-free; public items documented
- [x] Baselines recorded (if the package has performance gates) — IPC ping/propose criterion baseline `wp07b`
- [x] No new `TODO(` without a `WP-` reference; no new dependency without justification
- [x] Nothing written outside the repository except documented temp dirs (`/tmp/omacell-ipc-*` in tests)

### Acceptance (WP-07b)

- [x] Request/reply/event fixtures validate against the committed IPC v1 schemas and round-trip through client/server — `ipc_protocol.rs`, `ipc_server.rs`
- [x] Integration tests cover query, proposed mutation, explicit apply/revert, subscription, two concurrent clients, request ordering, timeouts, and shutdown of active clients — `ipc_server.rs`
- [x] Malformed, partial, deeply nested, oversized, unknown-version, unknown-field, and internal-command inputs are rejected without panic or server death — `ipc_protocol.rs`, `mutating_execute_is_rejected_internal_ids_are_rejected`
- [x] Socket directory/permissions, owner validation, stale cleanup, and symlink resistance are tested on Linux — `runtime_dir_rejects_symlink_and_world_writable`, `stale_socket_for_dead_pid_is_removed`, `leftover_socket_for_a_live_pid_without_a_listener_is_stale`
- [x] A stalled or selectively filtered subscriber cannot block mutations or overflow on irrelevant events — `stalled_subscriber_does_not_block_another_client`, `filtered_events_do_not_consume_the_subscriber_queue`
- [x] Decoder fuzz target is listed by `cargo fuzz list` and runs in the existing nightly job; review smoke 10,000 runs clean
- [x] Local request/reply benchmark records IPC overhead separately from recalculation — `ipc_roundtrip` ping ~10.5 µs, propose ~51.5 µs
