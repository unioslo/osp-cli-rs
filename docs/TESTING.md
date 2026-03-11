# Testing Strategy

`osp-cli-rust` uses behavior-first TDD with explicit behavior tiers plus a
small architecture guardrail suite.

## Test Layout

Root package tests are currently anchored in `tests/`:

```text
tests/
  architecture.rs
  unit.rs
  integration.rs
  contracts.rs
  e2e.rs
  architecture/
    *.rs
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

Internal unit tests also live next to the code they cover in `src/**/tests.rs`
and `src/**/tests/*.rs`.

## Tier Definitions

- `unit`
  - Pure logic and edge-case parsing.
  - No process spawning.
- `integration`
  - In-process flow across major subsystems.
  - Example: host assembly -> native/plugin dispatch -> renderer.
- `contracts`
  - Public CLI behavior through spawned binary.
  - Prefer isolated roots via `tests/contracts/test_env.rs`.
- `e2e`
  - Real process and terminal behavior where subprocess or PTY behavior matters.
  - Includes PTY-driven REPL flows and a few binary-surface smoke checks.
- `architecture`
  - Fast structural guardrails.
  - Example: import boundaries, public facade limits, and toolchain alignment.

## TDD Workflow (Required)

1. Add or adjust a failing `contracts` test for user-visible behavior.
2. Add one `integration` test when the behavior crosses subsystem seams.
3. Add `unit` tests only for branchy or failure-prone internals.
4. Add `e2e` only when the behavior depends on real process or PTY semantics.
5. Add `architecture` coverage when the change introduces or relaxes a structural policy.
6. Implement minimal code for green.
7. Refactor without changing behavior.

## Commands

- Root package: `cargo test`
- Architecture only: `cargo test --test architecture`
- CLI contracts only: `cargo test --test contracts`
- REPL/process e2e only: `cargo test --test e2e`
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

## Current Coverage Shape

- `contracts`
  - plugin discovery, provider selection, and dispatch
  - config, theme, doctor, help, history, version, and profile command surfaces
  - stdout/stderr separation and snapshot-backed visible output promises
- `integration`
  - app assembly, config load/mutate flows, plugin manager behavior, guide UI
  - DSL parse/eval flows and service execution with test seams
- `unit`
  - parser, completion, config, UI, plugin, and runtime helpers
- `e2e`
  - binary surface smoke coverage
  - PTY-driven REPL help, completion, intro, prompt, highlight, and plugin flows
- `architecture`
  - import limits
  - intent seams
  - curated root facade and pinned toolchain alignment

## Definition Of Done Per Feature

- [ ] Contract test covers user-facing behavior.
- [ ] Integration test covers cross-crate flow.
- [ ] Unit tests only where risk justifies them.
- [ ] `cargo test` passes at repo root.
