# ADR-004 — Scripting language is Lua

| | |
|---|---|
| Status | **Decided** |
| Date | 2026-08-27 |
| Spec | §6.10, §11.2 |
| Plan default (D5) | Lua 5.4 via `mlua` (vendored) |

## Decision

In-process scripting is Lua 5.4. Python is available to plugins through a
subprocess bridge (`omacell run --python`) rather than embedded, to keep the
binary small and the sandbox simple.
