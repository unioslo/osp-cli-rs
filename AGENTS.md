# AGENTS.md

Repo-specific guidance for the generic OSP CLI.

## OSP workspace knowledge

When this repo is checked out alongside the OSP workspace wiki, read
`../AGENTS.md` and `../.wiki/index.md` (paths relative to this file), then follow
only the topic links relevant to your task. Update durable discoveries in the
wiki with evidence; this repo's rules remain authoritative for local work.
If those workspace files are absent, use this repo's own docs and instructions.

## Engineering

Prefer simple code, narrow interfaces, existing dependencies, and small
reversible refactors. Centralize shared decisions rather than similar syntax.
Do not add per-bug regression or negative tests (see the workspace AGENTS.md);
prefer stable-boundary tests and useful public examples over overlapping tests
and noisy comments.

## Commit message contract (enforced in this repo)
- The installed hook enforces: `<type>(<scope>): <Subject>`
- Allowed `type`: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`,
  `style`, `ci`, `build`, `perf`, `revert`
- `scope` is optional; if present it must be lowercase and may use
  letters/digits plus `.`, `_`, `/`, and `-`
- `Subject` must start with uppercase, must not end with a period, and the
  full subject line must be 72 characters or fewer
- The line immediately after the subject must be blank before the body starts
- Bodies are required for ordinary commits; placeholder headings alone do not
  satisfy the hook
- Body lines must wrap at 80 columns
- Do not embed literal `\n` sequences in a single `-m`; use the editor or
  multiple `-m` flags instead
- Git-generated `Merge ...`, `Revert ...`, and autosquash `fixup! ...` /
  `squash! ...` subjects are allowed exceptions

Examples:
- `feat(cli): Add profile positional dispatch`
- `fix(config): Normalize profile and terminal keys`
- `ci(test): Pin Rust 1.94 and tighten test guardrails`

Commit template:

`<type>(<scope>): <Subject>`

`<blank line>`

`Why:`
`What:`
`Verification:`
`Refs:`

When filling the template, write real content under `Why`, `What`, and
`Verification`; do not leave them as empty headings.
