# ADR-006 — One skill format

| | |
|---|---|
| Status | **Decided** |
| Date | 2026-08-27 |
| Spec | §8.8, §11.2 |
| Plan default (D9) | `SKILL.md` directories, same layout as Omarchy's |

## Decision

In-app agent skills use the same `SKILL.md` layout as Omarchy's shipped
skill and the coding agents' skill directories, so a skill written for the
in-app agent also works when the workbook is handed to Claude Code, Codex,
Pi, or OpenCode.
