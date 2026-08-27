# ADR-002 — Engine: build or adopt

| | |
|---|---|
| Status | **Proposed** |
| Date | 2026-08-27 |
| Spec | §11.2, §11.3 |
| Spike | WP-S1 |
| Plan default (D2) | Build `omacell-core` per spec §11.3 |

## Context

The formula engine, dependency graph, dynamic arrays, `LET`/`LAMBDA`, and
async AI nodes are the product. IronCalc is a Rust, open-source,
Excel-compatible candidate.

## Options

1. **Build `omacell-core`** — full control over dynamic arrays, async AI
   nodes, and the 64-byte numeric-cell budget.
2. **Adopt IronCalc** — if license, LAMBDA/dynamic-array coverage, and
   1M-formula graph performance hold; contribute rather than fork.

## Decision

Proposed (for agent execution until WP-S1 reports): **build `omacell-core`**.
Revisit if WP-S1 shows IronCalc covers L1/L2 and the graph performance with
a compatible license.
