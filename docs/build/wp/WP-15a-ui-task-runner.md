# WP-15a — Non-blocking UI command task runner

| | |
|---|---|
| Phase | 3 — Surfaces I — config, CLI, UI core, TUI |
| Lane | C — Surfaces (UI runtime) |
| Size | M (≈ 4–6) |
| Depends on | WP-08, WP-15 |
| Unblocks | WP-16, WP-28, G3 |
| Spec sections | §10.2, §11.5 |
| Where | `crates/bus`, `crates/ui`, `crates/tui`, CLI composition root |

## Goal

Keep input, selection, scrolling, and paint responsive while recalc, file open/save, import, or export runs. Preserve the single-writer workbook rule and give both frontends one task/progress/cancellation contract.

## Deliverables

- Add a message-based command task runner around the live `Bus`. The worker is the only workbook writer; UI and IPC submit requests and consume copy-on-write reader snapshots/results instead of competing for a long-held mutex.
- Add additive task-state types outside the frozen WP-01 `Event` and WP-07b IPC envelopes: stable task id, command id, `queued|running|cancelling|completed|failed`, optional bounded progress `(done, total, label)`, and a cancellation handle.
- Classify long operations explicitly in the composition layer. Do not infer them from command-name prefixes or add a field to frozen `CommandDescriptor`.
- Wire existing WP-08 progress/cancellation into import. Add cooperative cancellation checkpoints for full recalc and file export/save where their current APIs lack them; cancellation must leave the live workbook/file in the operation's documented atomic state.
- Keep UI-local interaction responsive from reader snapshots while the writer is occupied. At minimum: arrows/data-edge navigation, selection/extend, wheel scroll, zoom, palette/panel open/close, resize, and quit/cancel.
- Render one bounded progress segment and stale snapshot state in the TUI. `Esc` cancels the focused cancellable task without closing unrelated panels; completed/failed results reconcile the latest snapshot and status message.
- Expose the same runner contract to WP-16. The GUI must reuse it rather than create a second scheduler or place the workbook behind a toolkit lock.

## Implementation notes

- Preserve request ordering for mutations. Cancellation is cooperative; never terminate a writer thread or expose a partially applied command transaction.
- Snapshot publication must be O(1) or copy-on-write with respect to populated cells. Do not clone a 20M-cell workbook per frame or progress update.
- Bounded channels only. A stalled frontend must not block the writer or grow task/progress queues without limit; coalesce intermediate progress for the same task.
- Existing synchronous `Bus::execute` remains available for CLI one-shot commands and tests. The runner is additive and owns the long-lived frontend path.
- Do not change the frozen core `Event`, command schema, or IPC envelope. If implementation proves that unavoidable, stop with an RFC in `reports/WP-15a.md` before changing code.

## Acceptance criteria

- [ ] With a deterministic 500 ms mock long command running, a 200×60 `TestBackend` continues to paint under 16 ms and navigation/selection/scroll/zoom each complete within 50 ms.
- [ ] TUI-started and IPC-started long commands use the same single writer; mutation order and exactly-once outcomes are proven under concurrent submit/cancel tests.
- [ ] Recalc, import, and export expose progress and cooperative cancellation. Tests prove cancelled import/recalc/export leave no partial live transaction or destination-file replacement.
- [ ] Task/progress/result queues are bounded, stalled subscribers cannot block the worker, and repeated progress for one task is coalesced.
- [ ] TUI tests cover running/completed/failed/cancelled status, `Esc` cancellation, continued input, terminal resize, and clean shutdown with a task in flight.
- [ ] WP-16's package names this runner as its required command/snapshot input; no toolkit owns or locks `Bus` directly.

## Tests

- Deterministic barrier-based concurrency tests; no sleeps as synchronization.
- `TestBackend` interaction and redraw benchmark while the writer is held by the mock task.
- Atomic-file and workbook-state cancellation tests for real recalc/import/export paths.
- Queue-capacity/stalled-consumer tests and clean runner shutdown tests.

## Procedure

1. Read `AGENTS.md`, this file, the listed spec sections, and the *Interfaces exposed* sections of `reports/WP-08.md` and `reports/WP-15.md`.
2. Write the *Plan* section of `reports/WP-15a.md` before code. Record the exact writer ownership, snapshot publication, task classification, and cancellation boundaries.
3. Create branch `wp/15a-ui-task-runner`.
4. Write the barrier-based concurrency and cancellation tests first; implement until they pass; run `just check` and the redraw bench.
5. Update WP-16's handoff notes if the final additive interface differs from this package's proposed names.
6. Complete the report and open a PR titled `WP-15a: Non-blocking UI command task runner`. Do not merge.

## Done when

Every acceptance box is ticked with evidence, CI is green, the report records redraw/input latency and cancellation behavior, and WP-16 can consume the runner without accessing a toolkit-owned `Mutex<Bus>`.
