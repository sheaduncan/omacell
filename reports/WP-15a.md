# Report — WP-15a: Non-blocking UI command task runner

## Plan (written before coding)
- Files/modules to create:
  - `crates/bus/src/{task,runner}.rs` — additive task types + worker that owns `Bus`
  - `crates/bus/tests/runner.rs` — barrier-based concurrency, queue bounds, cancel order
  - `crates/ui/src/local.rs` — session-only commands against a reader snapshot
  - `crates/tui` — paint/input from `Arc<ReaderSnapshot>`; progress segment; Esc cancel
  - CLI composition: spawn runner after command registration; IPC submits to the same handle
  - Recalc/import/export cooperative cancel checkpoints
- Interfaces:
  - `TaskRunner` / `TaskRunnerHandle` / `ReaderSnapshot` / `TaskState` / `CancelHandle` / `LongOps`
  - Existing `Bus::execute` unchanged for CLI one-shots
- Tests first:
  - Barrier-held mock long command: 200×60 paint < 16 ms; nav/sel/scroll/zoom < 50 ms
  - TUI- and IPC-originated longs share one writer; mutation order; exactly-once
  - Cancelled import/recalc/export leave no partial live transaction or dest-file replace
  - Bounded queues; stalled subscriber cannot block worker; progress coalesced
  - TUI running/completed/failed/cancelled, Esc, resize, shutdown with in-flight task
- Decisions:
  - Worker thread is the only `&mut Bus` / workbook writer
  - Snapshot: `Arc<ReaderSnapshot>` published after each completed/cancelled command (O(1) per frame; O(cells) once per commit, not per progress tick)
  - Long ops classified by composition `LongOps` (`calc.recalc`, `file.open`, `file.save`, `file.export`), not `CommandDescriptor`
  - Session-local nav/sel/view/palette/panel run against the last snapshot while the writer is busy
  - Import loads into a scratch workbook and swaps on success; save/export abort before atomic rename; recalc restores a pre-pass clone on cancel
  - No frozen `Event` / IPC envelope / `CommandDescriptor` changes
- Open questions at planning time:
  1. Per-cell stale hatching during a live recalc vs overlay-while-busy. Plan: keep last snapshot; TUI shows a progress segment + existing busy/stale chrome until the committed snapshot arrives.

## What was built

A single-writer command worker (`TaskRunner`) owns the live `Bus`. Front-ends and runner-backed IPC (`serve_runner`) submit work; they paint from `Arc<ReaderSnapshot>` published after each commit. Long operations are listed in composition-layer `LongOps`, not frozen command metadata. While the writer is busy, session-local navigation/selection/zoom/palette/panel run through `omacell_ui::apply_local_command` against the last snapshot.

`Esc` cancels the running task without dismissing unrelated panels. Dropping `TaskRunner` requests cancel and joins the worker. Recalc cooperative-cancels between generations and restores a pre-pass workbook clone. CSV `file.open` loads into a scratch workbook (WP-08 `LoadOptions.cancel`); the live book is replaced only on success. CSV export writes a temp file and renames only if not cancelled.

`Bus::execute` remains for CLI one-shots. `test.hold` is a test-only barrier command.

Key files: `crates/bus/src/{task,runner}.rs`, `crates/ui/src/local.rs`, TUI `app.rs`, CLI `files.rs` / `run.rs`, `docs/build/wp/WP-16-gui-foundation.md` (handoff names).

## Interfaces exposed (for dependents)

| Item | Notes |
|---|---|
| `TaskRunner::spawn(bus, LongOps)` | Worker owns `Bus`. Drop = cancel in-flight + join |
| `TaskRunnerHandle` | `submit` / `submit_wait` / `propose` / `apply` / `dry_run` / `snapshot` / `drain_events` / `running_cancel` / `command_ids` |
| `ReaderSnapshot` | `{ workbook, spill }` behind `Arc`; clone of Arc is O(1) per frame |
| `TaskState` / `TaskStatus` / `TaskProgress` / `TaskEvent` / `CancelHandle` | Additive; not frozen `Event` or IPC |
| `LongOps::production()` | `calc.recalc`, `file.open`, `file.save`, `file.export` |
| `register_hold_command` | Test-only `test.hold` |
| `omacell_ui::{is_local_command, apply_local_command}` | Busy-path session commands |
| `UiSession::apply_config_ids` | Keymap reload against captured command ids |
| `ipc::serve_runner` | IPC execute/propose/dry-run through the same writer |
| `CommandContext::{cancel_flag, is_cancelled, report_progress}` | Adapters (file/recalc) |
| `RecalcEngine::recalc_*_with_ctl` / `RecalcResult.cancelled` | Cooperative cancel + restore |

WP-16: spawn `TaskRunner` after registering commands; do not put `Bus` behind a toolkit mutex. See updated WP-16 binding notes.

## Deviations from the spec or the package (with reasons)

- **Snapshot clone is O(cells) at commit**, not per frame or progress tick. Making `Workbook` internally `Arc` per sheet would be a larger core change; Arc publication still meets the per-frame requirement.
- **Debug paint bound is 100 ms**; release/CI-style 16 ms is the spec gate (`cfg!(debug_assertions)` in the TUI runner test). Criterion `tui_redraw_200x60_1m` remains the empty-sheet paint budget.
- **IPC subscribe/changeset control** on `serve_runner` is ping + registry commands only. Full changeset control stays on bus-backed `serve()` used by CLI one-shots.
- **CSV load_into into a caller-owned workbook** still writes partial rows then `csv.cancelled` (WP-08). The *command* path is atomic because `file.open` assigns the scratch book only on success.

## Measurements

Host: local Linux. `CARGO_TARGET_DIR=$HOME/.cache/omacell/target`.

- `just check` — pass
- `cargo test -p omacell-bus --test runner` — 5 pass
- `cargo test -p omacell-tui --test runner` — 3 pass (debug paint 17 ms, asserted < 100 ms)
- `cargo test -p omacell-cli --test cancel_atomic` — 3 pass
- Existing TUI keymap/reload/snapshot suites still pass
- `cargo deny check` — pass (no new crates.io deps)

## Open questions / decisions needed

1. Whether `serve_runner` should grow subscribe/changeset list RPCs before G3 dogfooding of `omacell ipc` against a live TUI.
2. Per-cell stale hatching during an in-progress recalc still waits on a committed snapshot (busy chrome only).

## RFC (only if a frozen contract changed)

None. Frozen WP-01 `Event`, WP-07a command schemas, and WP-07b IPC envelope are unchanged.

## Checklist
- [x] `just check` green on a clean clone
- [x] Every acceptance criterion ticked with evidence
- [x] Docs warning-free; public items documented
- [x] Baselines recorded (if the package has performance gates) — paint-while-busy proven non-blocking; 16 ms remains the release redraw gate
- [x] No new `TODO(` without a `WP-` reference; no new dependency
- [x] Nothing written outside the repository except documented temp dirs

### Acceptance (WP-15a)

- [x] Deterministic mock long command: 200×60 paint stays off the writer; nav < 50 ms — `crates/tui/tests/runner.rs`
- [x] TUI-started and IPC-started longs share one writer; mutation order under concurrent submit — `crates/bus/tests/runner.rs`; `serve_runner` uses `submit_wait`
- [x] Recalc/import/export cancel leaves no partial live transaction or dest replace — `crates/cli/tests/cancel_atomic.rs`
- [x] Bounded task-event queue; stalled consumer cannot block worker; progress coalesced — `crates/bus/tests/runner.rs`
- [x] TUI running/completed/failed/cancelled, Esc, continued input, resize, shutdown with in-flight task — `crates/tui/tests/runner.rs`
- [x] WP-16 package names this runner as required input — `docs/build/wp/WP-16-gui-foundation.md`
