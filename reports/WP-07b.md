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
| `MAX_FRAME_BYTES` | 1,048,576 |
| `MAX_JSON_DEPTH` | 32 |
| `MAX_CONNECTIONS` | 32 |
| `MAX_EVENT_FILTERS` | 16 |
| `MAX_EVENT_QUEUE` | 64 |
| `MAX_EVENT_QUEUE_BYTES` | 262,144 |
| Socket directory mode | 0700 |
| Socket mode | 0600 |
| Default request timeout (client) | 5s |

## What was built

(To be filled after implementation.)

## Interfaces exposed (for dependents)

(To be filled after implementation.)

## Deviations from the spec or the package (with reasons)

## Measurements

## Open questions / decisions needed

## RFC (only if a frozen contract changed)

None planned. IPC v1 is a new freeze point.

## Checklist

- [ ] `just check` green on a clean clone
- [ ] Every acceptance criterion ticked with evidence
- [ ] Docs warning-free; public items documented
- [ ] Baselines recorded (if the package has performance gates)
- [ ] No new `TODO(` without a `WP-` reference; no new dependency without justification
- [ ] Nothing written outside the repository except documented temp dirs
