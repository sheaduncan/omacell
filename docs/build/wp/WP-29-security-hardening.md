# WP-29 — Public-repository security hardening

| | |
|---|---|
| Phase | Security hotfix |
| Lane | D — Integration |
| Size | M |
| Depends on | WP-05c, WP-07a, WP-08, G1 |
| Unblocks | Public development |
| Spec sections | §12, §15, §16 |
| Where | `crates/fn`, `crates/io`, `crates/bus`, `.github/workflows` |

## Goal

Close the denial-of-service and spreadsheet-export findings from the first audit of the public repository, and make third-party CI execution reproducible.

## Deliverables

- Replace recursive `SEARCH` wildcard matching with bounded, linear-time regex matching and adversarial regression tests.
- Bound CSV preview and clipboard parsing by bytes, rows, columns, cells, and field size.
- Stream CSV export one record at a time, bound the convenience in-memory exporter, and make handling of formula-like text explicit and safe by default.
- Bound command range expansion and the retained changeset count/serialized size.
- Pin third-party GitHub Actions to full commit SHAs and declare least-privilege workflow permissions.
- Record GitHub repository controls that require an owner setting change as WP-30.

## Acceptance criteria

- [x] Adversarial `SEARCH` inputs complete without recursive backtracking or stack growth.
- [x] CSV preview/clipboard requests fail with `csv.limit` before crossing documented bounds.
- [x] `export_write` has memory bounded by one row; `export` fails cleanly above its buffer limit.
- [x] Formula-like text is rejected by default and can be preserved or escaped only by explicit policy.
- [x] Oversized ranges and changeset stores fail with stable error codes.
- [x] Workflow `uses:` references are immutable SHAs and workflow permissions are read-only.
- [x] `just check` is green.

## Tests

- Unit and integration tests for wildcard adversarial cases, CSV limits/streaming/formula policy, range limits, and changeset budgets.
- `just check` and focused package test suites.

## Procedure

1. Write the Plan section of `reports/WP-29.md` before code.
2. Add failing regression tests for each code-path finding.
3. Implement the bounded behavior and document public constants/policies.
4. Pin workflow dependencies and write the WP-30 owner-setting action.
5. Run `just check`, complete the report, and open `WP-29: Public-repository security hardening`. Do not merge.

## Done when

All acceptance boxes are supported by tests or source inspection, the report records frozen-contract changes, and CI is green.
