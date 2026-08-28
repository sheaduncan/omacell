# Omacell build bundle

- `PLAN.md` — the logical plan: decisions, phases, gates, lanes, dependency graph, execution protocol.
- `AGENTS.md` — repository conventions agents must follow (copy to the repo root; `CLAUDE.md` can contain `@AGENTS.md`).
- `wp/` — one buildable markdown per work package (35 files).
- `templates/` — kickoff prompt, report template.
- `spec/omacell-design-spec.md` — the design specification (v0.3) the packages reference by section.

Install: copy this directory to `docs/build/` in the repository and `spec/omacell-design-spec.md` to `docs/spec/`. Start with `wp/WP-00-bootstrap.md`.
