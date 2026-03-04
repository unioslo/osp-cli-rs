# Contributing

## Commit Message Contract

This repository uses a strict commit subject format:

`<type>(<scope>): <Subject>`

- Allowed `type`:
  - `feat`, `fix`, `docs`, `refactor`, `test`, `chore`, `style`, `ci`, `build`, `perf`, `revert`
- `scope` is optional.
- `Subject` must start with uppercase and must not end with a period.
- Subject line max length: 72 chars.
- If a body is present, add one blank line after the subject.

Examples:

- `feat(cli): Add profile positional dispatch`
- `fix(config): Normalize profile and terminal keys`
- `docs(state): Clarify config revision invariants`

## Hook Setup

Run once per clone:

```bash
./scripts/install-git-hooks.sh
```

This sets:

- `core.hooksPath=.githooks`
- `commit.template=.gitmessage`

The commit hook is in `.githooks/commit-msg`.
