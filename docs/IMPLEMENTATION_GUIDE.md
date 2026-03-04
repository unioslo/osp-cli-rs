# OSP CLI Rust Implementation Guide

This guide defines implementation order and quality gates.
Rule: enforce architecture and test scaffolding before feature breadth.

## Read Order

1. `docs/ARCHITECTURE.md`
2. `docs/UI_ARCHITECTURE_CONTRACT.md`
3. `docs/TESTING.md`
4. `docs/CLI_PROFILE.md`
5. `docs/CONFIG.md`
6. `docs/LOGGING.md`
7. `docs/THEMES.md`
8. `docs/LDAP.md`
9. `docs/MVP_LDAP_REPL.md`
10. `docs/DSL.md`
11. `docs/UI.md`
12. `docs/FORMATTING.md`
13. `docs/REPL.md`
14. `docs/COMPLETION.md`
15. `docs/PLUGIN_PROTOCOL.md`
16. `docs/PLUGIN_PACKAGING.md`
17. `docs/STATE.md`
18. `docs/ROADMAP.md`
19. `docs/CONTRIBUTING.md`

## Ground Rules

- Keep one-shot and REPL workflows.
- Keep DSL pipeline support (`P`, `F`, `V`).
- Rename context -> profile in final UX.
- Defer login/auth for MVP.
- Prefer integration + contract tests over abundant unit tests.

## Workspace Crates (Target + Current)

- `osp-core`
- `osp-config`
- `osp-dsl`
- `osp-ports`
- `osp-api`
- `osp-services`
- `osp-ui`
- `osp-repl`
- `osp-cli` (binary `osp`)

## Phase Checklist

## Phase 0: Workspace + Boundaries

- [x] Workspace split into dedicated crates.
- [x] Compile-time crate dependency boundaries in place.
- [x] Architecture contract documented.
- [x] Test tiers structured (`unit`, `integration`, `contracts`, `e2e`).

## Phase 1: First Working LDAP MVP (Mocked)

- [x] `ldap user <uid>` one-shot works.
- [x] `ldap netgroup <name>` one-shot works.
- [x] REPL runs same commands.
- [x] `--filter` key/value works.
- [x] `--attributes` projection works.
- [x] DSL `P`, `F`, `V` works.
- [x] Output modes + color/unicode toggles implemented.
- [x] Contract/integration tests in place.

## Phase 2: Profile Parsing (No Login Yet)

- [x] Deterministic config resolver foundation in `osp-config`:
  profile/terminal scopes, source precedence, env mapping, interpolation.
- [x] Generic loader abstraction in `osp-config`:
  `ConfigLoader` trait + `LoaderPipeline`.
- [x] Typed schema/adaptation + strict unknown-key validation.
- [x] Secrets loaders integrated into the loader pipeline.
- [x] `osp` -> REPL using default profile.
- [x] `osp <profile>` -> REPL in that profile.
- [x] `osp <profile> <command...>` -> one-shot in profile.
- [x] `osp <command...>` -> one-shot in default profile.
- [x] Ambiguity tests for profile vs command token.

## Phase 2.5: Runtime State Split

- [x] Split runtime state into `ConfigState`, `UiState`, `ReplState`, `ClientsState`.
- [x] Route one-shot and REPL execution through `AppState`.

## Phase 2.6: State Hardening

- [x] Capture Python-to-Rust state migration plan in `docs/STATE.md`.
- [x] Add explicit `SessionState` for REPL-only runtime cache/pipeline artifacts.
- [x] Introduce explicit `RuntimeContext` (`profile_override` + `terminal_kind` + `$TERM`).
- [x] Add `ConfigState` revision + transactional replacement API.
- [x] Add contract tests proving session state does not leak into one-shot.
- [ ] Prepare `AuthState`/`OrchState` module seams (no implementation yet).

## Phase 3: One-Shot Config Surfaces

- [x] Add `osp config show|get|diagnostics`.
- [x] Include `--sources` and `--raw` views for config inspection.
- [x] Add `osp config explain <key>` with precedence and interpolation trace.

## Phase 3.5: Logging + Diagnostics Baseline

- [x] Add `miette` diagnostic reporting at CLI boundary.
- [x] Add global `-v/-q/-d` semantics in Rust CLI.
- [ ] Add tracing bootstrap (`stderr` + file sink) with config-driven levels.
- [ ] Add contract tests for stderr/stdout separation.

## Phase 3.6: UI Architecture Contract

- [x] Add explicit UI architecture ownership contract doc.
- [x] Refactor `osp-ui` to explicit format -> document -> renderer pipeline.
- [x] Add `osp-ui` tests for format selection and renderer behavior.
- [x] Add rich/plain renderer backends behind a stable formatter API.
- [x] Add grouped semantic message rendering with style-aware headers.
- [x] Add markdown (`--format md`) table rendering path.
- [x] Add width-aware table truncation in renderer.
- [x] Add plain clipboard render path through same formatter pipeline.

## Phase 3.7: Theme System

- [x] Add typed built-in theme registry in `osp-ui`.
- [x] Seed active theme from config with `--theme` override support.
- [x] Add `theme list|show|use` built-in commands (CLI + REPL).
- [x] Replace scattered ANSI literals with semantic style tokens + theme mapping.
- [ ] Add persistent theme writes once `config set` lands.

## Phase 3: Real LDAP Adapter

- [ ] Add real LDAP adapter in `osp-api` implementing `osp-ports` traits.
- [ ] Keep mock adapter for deterministic tests.
- [ ] Add adapter boundary integration tests.
- [ ] Add failure-mode tests (timeouts, invalid filter, auth failures).

## Phase 4: REPL UX Parity

- [ ] Rich completion tree (command/option/value aware).
- [ ] History parity (`Ctrl+R`, `!!`, `!53`, `!-N`, prefix recall).
- [ ] Prompt formatting hooks + profile-aware prompt.
- [ ] Un-ignore PTY e2e tests and stabilize in CI.

## Phase 5: Executable Plugin Framework

- [x] Add plugin manager in `osp-cli` for discovery and dispatch.
- [x] Implement protocol validation for `DescribeV1` and `ResponseV1`.
- [x] Add `osp plugins list|commands|enable|disable|doctor`.
- [x] Add bundled manifest support for internal distro packaging.
- [x] Add contract tests for plugin discovery order and conflict behavior.
- [x] Enforce command ownership: domain verbs are plugin-owned.
- [x] Add describe-cache-backed help/completion source for REPL.

## Gate Before Moving Phases

- [ ] `cargo test` passes at workspace root.
- [ ] No crate dependency rule violations.
- [ ] New user-facing behavior includes contract coverage.
- [ ] Docs updated in same PR.
