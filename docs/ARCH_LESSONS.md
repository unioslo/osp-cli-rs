# Architectural Lessons From osprov-cli

This document captures architectural debt in osprov-cli and specific fixes we
should apply in the Rust rewrite. It goes beyond surface issues and focuses on
coupling, hidden behavior, and long‑term maintainability.

## Config System

**What we see**

- Context (soon “profile”) is stabilized via multi‑pass resolution loops.
- Config values can be callables, which introduces hidden dependencies.
- Loader priority is “lower wins,” which is non‑intuitive and hard to audit.
- Secrets appear in defaults, which is unsafe and leaks into tests.
- Some keys used by UI and REPL are not in the schema (`ui.colors`, `ui.ascii_borders`).
- Alias expansion is embedded in the resolver, blending config with command parsing.

**Fix in Rust**

- Select profile first, then resolve config once. No stabilization loop.
- Forbid callables in config values. Derived values are computed in code.
- Use explicit “last wins” precedence list in docs and tests.
- No secret defaults. Secrets must come from env or secret store.
- Every key referenced in code must be in schema (strict by default).
- Move alias expansion into the command parsing layer.

## CLI and REPL Coupling

**What we see**

- REPL uses Click/Typer in standalone mode and traps `sys.exit`.
- `with_formatting` mutates function signatures at runtime.
- REPL relies on global mutable state (`state.regexes`, `_result_cache`).
- There is a command‑parsing hack (`_merge_orch_os_tokens`) to reshape args.
- Command discovery is dynamic and uses an internal “disabled modules” list.

**Fix in Rust**

- Use a dispatcher that never exits; return explicit `ExitCode`.
- Keep flags in one place (the CLI parser), not per‑command decorators.
- Replace `state.regexes` with an explicit `Pipeline` field on the dispatcher.
- Remove token merge hacks by designing the CLI grammar properly.
- Replace dynamic discovery with explicit command registration.

## DSL Architecture

**What we see**

- Two parsing paths exist (`core.pipeline` and `dsl.engine`).
- Unknown verbs fall back to quick search, which hides errors.
- Emoji separators leak into data and tests.
- Streaming and materialized execution are mixed without clear boundaries.
- Fuzzy key resolution can silently pick ambiguous keys.
- Quick search and filter share overlapping logic with subtle differences.

**Fix in Rust**

- One parser and a typed AST.
- Unknown verbs should error unless explicitly configured.
- ASCII‑first path syntax with optional legacy normalization.
- Stage kinds are explicit: streaming or materializing.
- Ambiguous keys error by default, with an optional “best match” mode.
- Consolidate key resolution into a single resolver used by all stages.

## UI and Formatting

**What we see**

- UI executes DSL, which mixes concerns.
- `ui.colors` and `ui.ascii_borders` are not in schema.
- `AppInterfaceMode` is overloaded to mean “no color.”
- Render mode auto‑selection uses hidden heuristics.
- Clipboard copy uses a plain renderer implicitly.

**Fix in Rust**

- Dispatcher runs DSL, UI only renders IR.
- Explicit knobs: `ui.render.mode`, `ui.color.mode`, `ui.unicode.mode`.
- Remove `AppInterfaceMode`; use clear rendering toggles.
- Make auto heuristics configurable and documented.
- Clipboard behavior is explicit and documented.

## Completion System

**What we see**

- Completion is driven by a large metadata tree with magic `__meta__` keys.
- Suggestions are based on Typer type hints, which can drift from runtime behavior.
- Orchestrator completion depends on OpenAPI calls done lazily inside REPL.
- Completion parser is not the same as the CLI parser.

**Fix in Rust**

- Use typed nodes for completion, no magic meta keys.
- Completion schema is generated from the same command registry as the parser.
- Dynamic hints are isolated behind a single augmentor with caching.
- Completion parsing reuses the same tokenizer as the CLI.

## State and Boundaries

**What we see**

- `State` owns config, UI, API clients, history, caches, and REPL state.
- Many ad‑hoc attributes are added at runtime.
- Testing seams are provided via global wrappers (e.g., `_HttpxProxy`).

**Fix in Rust**

- Split state into explicit sub‑contexts: `Config`, `AuthState`, `Clients`, `UiState`, `ReplState`.
- Use typed structures for REPL‑only state.
- Prefer dependency injection traits over global wrappers.

## Command Registry

**What we see**

- Dynamic module discovery and priorities introduce ordering surprises.
- Feature gating is implicit via `_disabled_modules`.

**Fix in Rust**

- Explicit command registration order in one place.
- Feature flags are explicit and controlled via config or compile‑time flags.

## Observability and Error Handling

**What we see**

- UI errors and CLI errors share some paths but not all.
- Unknown config keys can silently pass if not in schema.

**Fix in Rust**

- One error classification layer with consistent exit codes.
- Schema rejects unknown keys by default; `extensions.*` is the only escape.

## Compatibility Guidance

We should preserve user‑facing behavior, but not hidden hacks. The rewrite should:

- Keep command names, flags, and DSL verbs stable.
- Keep REPL history shortcuts (`!!`, `!-N`, `!123`, `!prefix`).
- Keep `auto` format and render mode logic, but make it explicit.
- Keep current formatting and DSL output shapes.

## Migration Guidance

These fixes can be phased in:

1. Clean config and profile resolution.
2. New DSL parser + stage model.
3. New dispatcher + UI boundary.
4. REPL and completion on the new parser.
