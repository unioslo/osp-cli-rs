# Testing Strategy

`osp-cli-rust` uses behavior-first TDD with explicit behavior tiers plus a
small architecture guardrail suite.

The main rule is simple:

- start from the user-visible behavior
- choose the highest boundary that can own that behavior cheaply
- add lower-level tests only when they protect a real invariant or failure mode

Tests should read like promises, not like coverage storage.

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

## Ownership Heuristics

- One user-visible promise should have one primary owner tier.
- Prefer the highest boundary that can express the behavior without excessive
  setup or flake.
- When an outer-boundary test already owns a promise, keep inner tests only for
  local invariants, parser edges, or failure paths that the outer test does not
  explain well.
- Do not add many local tests that restate the same visible behavior with
  slightly different setup.

Default owners:

- `contracts`
  - visible CLI behavior
  - stdout and stderr boundaries
  - help, error, config, plugin, theme, doctor, history, and profile surfaces
- `integration`
  - in-process flows across config, app assembly, plugin manager, guide UI, and
    DSL evaluation seams
  - multi-step behavior that does not need real PTY semantics
- `e2e`
  - real binary, subprocess, PTY, prompt redraw, completion menu, and terminal
    behavior
- `unit`
  - branch-heavy transforms, reducers, parsers, classifiers, and awkward
    failure handling
- `architecture`
  - import rules, facade limits, and structural policies

## Test Selection Heuristics

Ask these questions in order:

1. Would a user notice this from the CLI surface?
   - Start with `contracts`.
2. Does the behavior cross subsystem seams in process?
   - Add one `integration` test.
3. Does the behavior depend on a real process, PTY, or terminal redraw?
   - Add `e2e`.
4. Is the real risk inside a parser, reducer, matcher, or failure branch?
   - Add `unit`.
5. Did the change introduce or relax a structural dependency rule?
   - Add `architecture`.

Short version:

- visible promise -> `contracts`
- cross-subsystem flow -> `integration`
- real terminal/process semantics -> `e2e`
- local invariant or branchy logic -> `unit`
- structural rule -> `architecture`

## TDD Workflow (Required)

1. Add or adjust a failing `contracts` test for user-visible behavior.
2. Add one `integration` test when the behavior crosses subsystem seams.
3. Add `unit` tests only for branchy or failure-prone internals.
4. Add `e2e` only when the behavior depends on real process or PTY semantics.
5. Add `architecture` coverage when the change introduces or relaxes a structural policy.
6. Implement minimal code for green.
7. Refactor without changing behavior.

For bugs:

1. Reproduce the bug at the highest boundary that a user would observe.
2. Only drop inward if that test would be too slow, too flaky, or too indirect.
3. If a fix adds an outer-boundary regression test, trim overlapping local tests
   when they no longer add signal.

## Commands

- Root package: `cargo test`
- Static confidence lane: `python3 scripts/confidence.py static`
  - formatting, lint, environment, and architecture checks
- Local confidence lane: `python3 scripts/confidence.py local`
  - repo-wide public docs, static checks, contracts, and integration
- Behavior-focused lane: `python3 scripts/confidence.py behavior`
  - contracts and integration without PTY-heavy `e2e`
- Full confidence lane: `python3 scripts/confidence.py full`
  - repo-wide public docs, static checks, unit, behavior, `e2e`, and full coverage
- Pre-push approximation: `python3 scripts/confidence.py pre-push`
  - local lane plus fast changed-file coverage
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

## Behavior Over Coverage

Coverage is a backstop, not the main confidence signal.

- `contracts`, `integration`, and a small `e2e` layer provide the main behavior
  confidence
- `unit` tests provide speed and precision for risky internals
- coverage helps catch forgotten testing, but it does not replace behavior
  ownership

When deciding whether to add a test, prefer:

- one strong contract or integration test

over:

- several local tests that all restate the same observable promise

## Coverage and Hooks

`TESTING.md` describes test shape and workflow. Coverage policy and git hook
behavior live in [CONTRIBUTING.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/CONTRIBUTING.md).

In short:

- install hooks with `./scripts/install-git-hooks.sh`
- `pre-commit` runs `public-docs.py --staged` and `confidence.py static`
- `pre-push` runs `confidence.py pre-push`
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

## Path-Based Defaults

Use these defaults unless a specific behavior clearly belongs elsewhere:

- `src/config/**`
  - `contracts` for visible config commands
  - `integration` for load, mutate, reload, and explain flows
  - `unit` for schema adaptation and parsing edges
- `src/plugin/**`
  - `contracts` for discovery, help, selection, and dispatch surface
  - `integration` for manager and environment propagation
  - `e2e` only for real subprocess behavior
- `src/repl/**`
  - `integration` for host, session, and transcript-like behavior without PTY
  - `e2e` for PTY and prompt/completion redraw semantics
  - `unit` for editor state machine internals
- `src/ui/**`
  - `contracts` for visible output promises
  - `unit` for layout and rendering algorithms
- `src/dsl/**`
  - `integration` for end-to-end pipeline behavior
  - `unit` for parser and verb edge cases

## Avoiding Duplicate Tests

- If a contract test already proves a help or output promise, do not restate the
  same promise in several local render tests.
- If an integration test already proves a config or plugin flow, keep only the
  unit tests that protect the tricky branches inside that flow.
- Keep `e2e` small. Do not promote a case to PTY coverage just because it is
  important.
- Prefer deleting overlap over accumulating parallel assertions at multiple
  layers.

## Definition Of Done Per Feature

- [ ] Contract test covers user-facing behavior.
- [ ] Integration test covers cross-crate flow.
- [ ] Unit tests only where risk justifies them.
- [ ] `cargo test` passes at repo root.
