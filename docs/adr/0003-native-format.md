# ADR-003 — Native format is `.xlsx`

| | |
|---|---|
| Status | **Decided** |
| Date | 2026-08-27 |
| Spec | §6.9, §11.2 |

## Decision

`.xlsx` (OOXML) is the native save format. `.omc` is a sibling for text
workflows, not a replacement. Round-trip fidelity is a v1 goal; L3 unknown
parts are preserved and re-emitted.
