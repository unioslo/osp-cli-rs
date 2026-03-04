# Testing Strategy

`osp-cli-rust` uses behavior-first TDD with explicit tiers.
This mirrors `osprov-cli`: `unit`, `integration`, `contracts`, `e2e`.

## Test Layout

Workspace tests are currently anchored in `crates/osp-cli/tests/`:

```text
crates/osp-cli/tests/
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

- Full workspace: `cargo test`
- CLI contracts only: `cargo test -p osp-cli --test contracts`
- DSL crate only: `cargo test -p osp-dsl`
- Services crate only: `cargo test -p osp-services`

## Current MVP Coverage (LDAP Mock + DSL + REPL)

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
- [ ] `cargo test` passes at workspace root.
