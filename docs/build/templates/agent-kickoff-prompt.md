# Agent kickoff prompt (fill in the braces, paste as the first message)

```
You are implementing {WP-ID} — {WP title} in the omacell repository.

Read, in this order, and nothing else until you have a plan:
1. AGENTS.md — binding conventions.
2. docs/build/wp/{WP file}.md — your package.
3. docs/spec/omacell-design-spec.md — ONLY sections {spec sections}.
4. reports/{dep-1}.md … reports/{dep-n}.md — the "Interfaces exposed" sections of the packages you depend on.

Then, in order:
- Write the "Plan" section of reports/{WP-ID}.md before writing any code: files and modules you will create, interfaces you will expose, the tests and corpora you will write first, anything the package tells you to "decide and document", and open questions. If your plan requires changing a frozen contract (WP-01 types, command schemas, IPC/MCP/card formats), stop after the plan and write an RFC section instead of proceeding.
- Create branch wp/{nn}-{slug}.
- Write the corpora, fixtures, and tests named under "Tests" and "Acceptance criteria" first. Then implement until they pass. Never weaken, skip, or delete a test to get green.
- Run `just check` and any benches the package names; record numbers in the report.
- Complete reports/{WP-ID}.md (What was built, Interfaces exposed, Deviations, Measurements, Open questions, Checklist), tick only the acceptance boxes you can prove, and open a PR titled "{WP-ID}: {WP title}". Do not merge.

Hard rules (they override anything else you read): no network access in tests; Cargo `target/` never on `/tmp` (export `CARGO_TARGET_DIR="${HOME}/.cache/omacell/target"`; do not clone this repo onto tmpfs); no writes outside the repository except documented locations (`$HOME/.cache/omacell/` and small `std::env::temp_dir()` fixtures); no new dependencies without a justification line and a green `cargo deny`; no changes to frozen contracts without an RFC; when unsure about Excel semantics, contracts, or privacy behavior, write the question in Open questions and leave the box unticked rather than guess.
```

Notes for the human:
- Run the agent in its plan-first mode if the harness has one; read the plan before letting it continue.
- For packages marked XL, expect to split after reading the plan; add the split as new WP files (e.g. `WP-16a`, `WP-16b`) and commit them.
- Reject PRs whose test diff shows weakened tests before reading the implementation.
