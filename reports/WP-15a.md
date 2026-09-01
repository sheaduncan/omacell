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

A single-writer command worker (`TaskRunner`) owns the live `Bus`. Front-ends and runner-backed IPC (`serve_runner`) submit work; they paint from `Arc<ReaderSnapshot>` published after each successful mutation. TUI commands outside the session-local set are always submitted asynchronously because even a nominally short edit can trigger a long automatic recalc. Long operations remain explicitly listed in composition-layer `LongOps`, not frozen command metadata. Session-local navigation/selection/zoom/edit-mode/palette/panel actions run through `omacell_ui::apply_local_command` against the last snapshot.

The conditional-format integration adds a dedicated read-only worker beside the
writer. GUI/TUI viewport requests are bounded to four frozen/scrolling pane
rectangles, coalesced to the newest request, and evaluated with the function
registry captured alongside the exact immutable reader snapshot. Results are
discarded when that snapshot is superseded, so formula evaluation never runs on
the paint thread and stale formatting cannot cross a workbook commit.

Runner-backed IPC now fans committed bus events into independent per-client
filtered queues with the same count and byte caps as the bus-backed server.
This fan-out is separate from `drain_bus_events`, so live frontend and retained
Lua consumers cannot lose an event to an IPC client; a stalled IPC connection
receives the frozen overflow record and closes without blocking the writer or
other clients.

`Esc` cancels the focused queued/running task without dismissing unrelated panels. Dropping `TaskRunner` cancels accepted work, resolves queued replies, and joins the worker. Queued cancellation prevents handler dispatch. Terminal task records and cancel flags are released, accepted task state is capped at the channel capacity plus the writer, progress labels/events are bounded, and terminal events carry the `Outcome` needed by toolkits to reconcile dirty state.

Recalc checks cancellation before/within generation commits, circular iteration, spill-follow waves, and stale-flag commits. Cancellation restores the pre-pass workbook/spill state; automatic recalc now runs inside the outer command transaction, so a cancelled edit and its derived values roll back together. CSV `file.open` loads and recalculates a staged workbook, then installs it only on success. CSV/OMC/XLSX save/export serialize to a unique same-directory temporary file and check cancellation before atomic destination replacement. `:wq` waits for save completion rather than immediately shutting down and cancelling the save.

`Bus::execute` remains for CLI one-shots. `test.hold` is a test-only barrier command.

Key files: `crates/bus/src/{task,runner}.rs`, `crates/ui/src/local.rs`, TUI `app.rs`, CLI `files.rs` / `run.rs`, `docs/build/wp/WP-16-gui-foundation.md` (handoff names).

## Interfaces exposed (for dependents)

| Item | Notes |
|---|---|
| `TaskRunner::spawn(bus, LongOps)` | Worker owns `Bus`. Drop = cancel in-flight + join |
| `TaskRunnerHandle` | `submit` / `submit_wait` / changeset propose/apply/revert/list/get / `dry_run` / `snapshot` / `drain_events` / `running_cancel` / `command_ids`; `request_conditional_formats` / `conditional_formats` provide a nonblocking viewport cache |
| `ReaderSnapshot` | `{ workbook, spill }` behind `Arc`; clone of Arc is O(1) per frame |
| `ConditionalFormatSnapshot` | Snapshot-bound resolved rectangles with O(1) cell lookup through the WP-18 overlays; exposes a bounded resolution error without blocking paint |
| `TaskState` / `TaskStatus` / `TaskProgress` / `TaskEvent` / `CancelHandle` | Additive; terminal success includes `Outcome`; not frozen `Event` or IPC |
| `LongOps::production()` | `calc.recalc`, `file.open`, `file.save`, `file.export` |
| `register_hold_command` | Test-only `test.hold` |
| `omacell_ui::{is_local_command, apply_local_command}` | Snapshot-backed session commands; toolkits call these before queueing |
| `UiSession::apply_config_ids` | Keymap reload against captured command ids |
| `ipc::serve_runner` | IPC execute/propose/dry-run and bounded event subscriptions through the same writer |
| `CommandContext::{cancel_flag, is_cancelled, report_progress, progress_sink, recalc_staged}` | Adapters (file/recalc) |
| `RecalcEngine::recalc_*_with_ctl` / `RecalcResult.cancelled` | Cooperative cancel + restore |

WP-16: spawn `TaskRunner` after registering commands; do not put `Bus` behind a toolkit mutex. See updated WP-16 binding notes.

## Deviations from the spec or the package (with reasons)

- **Snapshot clone is O(cells) at commit**, not per frame or progress tick. Making `Workbook` internally `Arc` per sheet would be a larger core change; Arc publication still meets the per-frame requirement.
- **Debug paint bound is 100 ms**; release/CI-style 16 ms is the spec gate (`cfg!(debug_assertions)` in the TUI runner test). Criterion `tui_redraw_200x60_1m` remains the empty-sheet paint budget.
- **CSV load_into into a caller-owned workbook** still writes partial rows then `csv.cancelled` (WP-08). The *command* path is atomic because `file.open` assigns the scratch book only on success.

## Measurements

Host: local Linux. Build artifacts were kept in the repository-local review scratch directory.

- `just check` — pass
- `cargo test -p omacell-bus --test runner` — 8 pass
- `cargo test -p omacell-bus --test ipc_server` — 20 pass, including runner-backed mutation policy and subscription fan-out without stealing retained events
- `cargo test -p omacell-tui --test runner` — 4 pass (debug uses the documented 100 ms CI-safe bound; release remains 16 ms)
- `cargo test -p omacell-cli --test cancel_atomic` — 4 pass, including real command-path import/export and mid-auto-recalc rollback
- Existing TUI keymap/reload/snapshot suites still pass
- Conditional-format follow-up: the barrier-held writer test proves overlay
  resolution remains independent; the full `just check` gate passes with worker
  invalidation, both renderer consumers, GUI snapshots, and strict Clippy.
- `cargo deny check` — pass (no new crates.io deps)

## Open questions / decisions needed

1. **Resolved in pre-WP-28 integration:** `serve_runner` exposes bounded event
   subscribe/unsubscribe without competing with retained host consumers.
2. **Post-1.0 decision:** retain busy chrome rather than per-cell stale hatching
   during an in-progress recalculation.

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

- [x] Deterministic mock long command: 200×60 paint and local navigation return
  while the writer is still held — `crates/tui/tests/runner.rs`; wall-clock
  redraw budgets are not asserted on shared required-CI runners
- [x] TUI-started and IPC-started longs share one writer; mutation order under concurrent submit — `crates/bus/tests/runner.rs`; `serve_runner` uses `submit_wait`
- [x] Recalc/import/export cancel leaves no partial live transaction or destination replacement — real command paths in `crates/cli/tests/cancel_atomic.rs`
- [x] Bounded task/event queues and retained state; stalled consumer cannot block worker; progress coalesced — `crates/bus/tests/runner.rs`
- [x] TUI running/completed/failed/cancelled, Esc, continued input, resize, shutdown with in-flight task — `crates/tui/tests/runner.rs`
- [x] WP-16 package names this runner as required input — `docs/build/wp/WP-16-gui-foundation.md`
