# REPL Quality Lift Plan

Goal: raise the floor on REPL code quality, maintainability, and readability
without turning the Rust REPL into a transliteration of Python manager debt.

The target is a clearer split:

- `osp-repl` is a terminal engine
- `osp-cli` owns REPL product policy
- shared REPL semantics come from typed models, not duplicated string logic

## Intent To Preserve

- REPL and one-shot CLI dispatch the same command surface
- intro/help/prompt sequencing is part of the UX contract
- shell-style scopes like `ldap` / `orch` are a real feature
- completion is context-aware and side-effect-free at the core
- history is a user feature with stable search/recall behavior

## Architecture Targets

- [ ] Add a typed `ReplSurface` model for visible commands, aliases, help rows, and completion inputs
- [ ] Split `repl/mod.rs` into product seams: surface, scope, presentation, dispatch, session
- [ ] Move host policy out of `osp-repl::run_repl()` and keep `osp-repl` focused on terminal mechanics
- [ ] Replace stringly `shell_stack` prefixing with a typed scope/frame model
- [ ] Unify completion, highlighting, and execution around one parsed REPL input shape
- [ ] Unify rich and basic fallback loops around one submit pipeline
- [ ] Keep history store boring and move scope/recording policy to the host
- [ ] Consolidate intro/help/prompt rendering into one presentation policy

## Concrete Work

### Surface

- [x] Extract one builder for root commands, aliases, overview rows, and completion specs
- [x] Remove duplicated built-in command assembly between overview and completion
- [ ] Add tests for built-in visibility and alias inclusion

### Session

- [ ] Isolate session orchestration from line dispatch
- [ ] Make restart/reload behavior explicit instead of open-coded in the loop
- [ ] Reduce `run_repl()` host-specific behavior

### Scope

- [ ] Model root vs subshell scope explicitly
- [ ] Derive prompt indicator, history scope, and prefixing from scope frames
- [ ] Keep repeated shell entry/exit behavior in one place

### Presentation

- [ ] Move intro, overview, and prompt rendering under one module
- [ ] Keep prompt templating pure and testable
- [ ] Add sequencing tests for intro/help/prompt behavior

### Completion

- [ ] Keep one source of truth for REPL-visible commands and DSL help surface
- [ ] Move toward one parsed input model shared by completer, highlighter, and dispatch
- [ ] Add tests for completion/highlight/dispatch agreement on partial lines

### History

- [ ] Separate storage from recording/filter/scope policy
- [ ] Add tests for shell-scoped history behavior and bang expansion
- [ ] Revisit whether history expansion belongs in `osp-repl` or the host

## Current Slice

- [x] Introduce `ReplSurface`
