# WP-30 — GitHub repository security controls

| | |
|---|---|
| Phase | Security follow-up |
| Lane | Maintainer operations |
| Size | S |
| Depends on | WP-29 |
| Unblocks | Stronger public contribution policy |
| Where | GitHub repository settings and rulesets |

## Goal

Finish the controls that require repository-owner decisions or GitHub settings and therefore cannot be safely completed in the WP-29 code PR.

## Current state (verified 2026-08-28)

- Secret scanning, push protection, Dependabot security updates, read-only default workflow permissions, and the active `main` ruleset are enabled.
- The `main` ruleset requires an up-to-date `check` status and pull requests, and blocks deletion/non-fast-forward pushes.
- Code scanning default setup is not configured.
- Actions may use any publisher and GitHub does not require full-length SHA pins.
- Secret-scanning non-provider patterns and validity checks are disabled.
- The ruleset requires zero approvals, does not require thread resolution, and grants an always-on repository-role bypass.

## Owner actions

1. Enable CodeQL default setup with the `security-extended` query suite for every detected language (`rust`, `actions`, and `python`). Wait for a successful default-branch scan, then add CodeQL as a required ruleset result at the `errors` alert threshold.
2. Require full-length commit SHA pins for Actions. Keep the WP-29 pins current through Dependabot; if publisher restrictions are enabled, explicitly allow only GitHub-owned actions plus `dtolnay/rust-toolchain`, `Swatinem/rust-cache`, and `taiki-e/install-action`.
3. Enable secret-scanning validity checks and non-provider patterns. Keep push protection enabled.
4. Add at least one trusted reviewer, then change the `main` ruleset to require one approval, approval of the last push by someone other than its author, and resolution of review threads.
5. Replace the always-on administrator bypass with the narrowest emergency bypass the maintainers can support. Record who may use it and require an issue or incident note after use.
6. Configure maintainer Git clients to use GitHub `noreply` addresses and sign new release/main commits. Decide separately whether to require signed commits; do not rewrite published history merely to remove the old address.

## Acceptance checks

- Repository code-scanning default setup reports `configured`, uses `security-extended`, and the default branch has no open high-or-higher alerts.
- Actions settings report `sha_pinning_required: true`; a test PR using a tag-only third-party action is rejected.
- Secret-scanning settings report push protection, validity checks, and non-provider patterns as enabled.
- The active `main` ruleset requires one approval, last-push approval, review-thread resolution, the `check` status, and CodeQL; deletion and non-fast-forward rules remain active.
- The ruleset has no undocumented always-on bypass.

## Done when

The acceptance checks are captured in `reports/WP-30.md` with redacted `gh api` output and a maintainer confirms the reviewer/bypass choices.
