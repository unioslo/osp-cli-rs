# Testing Strategy

`osp-cli-rust` uses behavior-first TDD with explicit tiers.
This mirrors `osprov-cli`: `unit`, `integration`, `contracts`, `e2e`.

## Test Layout

Root package tests are currently anchored in `tests/`:

```text
tests/
  unit.rs
  integration.rs
  contracts.rs
  e2e.rs
  unit/
    *.rs
  integration/
    *.rs
  contracts/
    *.rs
  e2e/
    *.rs
  fixtures/
    README.md
```

Per-crate internal unit tests may also exist in `src/lib.rs` modules
(e.g. `osp-dsl`, `osp-ports`, `osp-services`, `osp-repl`, `osp-api`).

## Tier Definitions

- `unit`
  - Pure logic and edge-case parsing.
  - No process spawning.
- `integration`
  - Cross-crate in-process flow.
  - Example: mock LDAP -> DSL -> renderer.
- `contracts`
  - Public CLI behavior through spawned binary.
- `e2e`
  - Interactive/PTY behavior.
  - Can remain `#[ignore]` until PTY harness is stable.

## TDD Workflow (Required)

1. Add/adjust a failing `contracts` test for user-visible behavior.
2. Add one `integration` test when behavior crosses crate boundaries.
3. Add `unit` tests only for branchy/error-prone internals.
4. Implement minimal code for green.
5. Refactor without changing behavior.

## Commands

- Root package: `cargo test`
- CLI contracts only: `cargo test --test contracts`
- Review snapshots: `cargo insta review`
- Re-record snapshots during a focused run: `cargo insta test -p osp-cli`

## Snapshot Placement

Keep snapshots close to the behavior they lock:

- use unit-style snapshots for stable single-command rendering, help output, and
  formatter chrome
- use contract snapshots for spawned CLI behavior where stdout/stderr boundaries
  matter
- use PTY or transcript-style tests for multi-step REPL flows, prompt redraws,
  and shell-state transitions

The current REPL snapshot coverage should stay mostly in unit tests. Move a case
to a transcript-style test only when the behavior depends on multiple commands or
interactive state over time.

## Coverage and Hooks

`TESTING.md` describes test shape and workflow. Coverage policy and git hook
behavior live in [CONTRIBUTING.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/CONTRIBUTING.md).

In short:

- install hooks with `./scripts/install-git-hooks.sh`
- keep `pre-commit` fast
- coverage is enforced on `pre-push`
- run `just cov` or `just cov-gate` when you need the full coverage check
- use `cargo insta review` to accept intentional output snapshot changes

## Current Coverage (LDAP Mock + DSL + REPL)

- `contracts`
  - `ldap` and `mreg` domain commands dispatched via executable plugins.
  - `osp <plugin-command> --help` and `osp <plugin-command> help` pass through.
  - `osp plugins list|commands|enable|disable|doctor`.
  - bundled manifest enforcement and mismatch detection.
- `integration`
  - `P`, `V`, `F` pipelines over LDAP fixtures.
- `unit`
  - DSL parser quote behavior.
  - LDAP filter key/value semantics.
- `e2e`
  - REPL smoke placeholder (ignored for now).

## Definition Of Done Per Feature

- [ ] Contract test covers user-facing behavior.
- [ ] Integration test covers cross-crate flow.
- [ ] Unit tests only where risk justifies them.
- [ ] `cargo test` passes at repo root.
