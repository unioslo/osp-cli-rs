# Roadmap and Feature Checklist

This is the incremental plan. Each phase should end with green contract and
integration tests.

## Phase 0 - Project scaffold

- [ ] Workspace structure created
- [ ] Directory rules documented
- [ ] Basic error type and exit codes
- [ ] Architectural lessons captured and agreed

## Phase 0.5 - MVP LDAP REPL

- [ ] REPL loop with prompt
- [ ] LDAP mock client
- [ ] `ldap user` command
- [ ] `ldap netgroup` command
- [ ] DSL pipes applied after LDAP output
- [ ] Auto format (single row -> MREG, multi -> table)
- [ ] Color/unicode toggles honored
- [ ] Contract + integration tests for LDAP

## Phase 1 - Profile and config

- [x] Config loader foundation (TOML + env + scoped layers)
- [x] Abstract loader pipeline (`ConfigLoader` + `LoaderPipeline`)
- [x] Typed schema adaptation + strict unknown key validation
- [x] Secrets loaders in pipeline (file + env)
- [x] Deterministic profile resolver
- [x] CLI parsing with profile rules
- [x] Contract tests for profile behavior
- [x] Remove config callables and secret defaults
- [x] Add `config explain` (winner/source/scope/candidates/interpolation)

## Phase 2 - DSL core

- [ ] Tokenizer and parser
- [ ] AST and executor
- [ ] Minimal verbs: F, P, S, G, A, L, C, Z
- [ ] Integration tests for DSL pipelines

## Phase 3 - One-shot CLI

- [ ] Command registry
- [ ] Output formatting pipeline skeleton
- [x] Basic config commands (`config show|get|explain|diagnostics`)
- [ ] Contract tests for CLI surface

## Phase 4 - REPL

- [ ] REPL loop using CLI dispatcher
- [ ] History support
- [ ] Basic completion
- [ ] E2E tests for REPL startup and exit

## Phase 4.5 - State hardening

- [x] State migration plan documented (`docs/STATE.md`)
- [x] Add explicit `SessionState`
- [x] Add explicit `RuntimeContext`
- [x] Add config revision + transaction seams
- [x] Add contracts for state isolation (REPL vs one-shot)
- [ ] Prepare `AuthState` and `OrchState` seams

## Phase 5 - Orchestrator parity (subset)

- [ ] Auth status
- [ ] Task info and find
- [ ] Watch task (streaming)
- [ ] Contract tests for orch surface

## Phase 6 - UI and formatting parity

- [ ] Table and JSON outputs
- [ ] Rich output parity for key commands
- [ ] Clipboard support
- [ ] Color mode: auto/always/never
- [ ] Unicode mode: auto/always/never (ASCII fallback)

## Phase 7 - Extended commands

- [ ] LDAP basic queries
- [ ] MREG host lookup
- [ ] Nivlheim host lookup
