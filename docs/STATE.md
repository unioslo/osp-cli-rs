# State Implementation Plan

This document plans the Rust `State` model based on a deep read of
`osprov-cli` state/config/runtime paths.

## Why This Matters

The Python `State` class currently owns too many concerns in one place:

- config loading + mutation + validation
- UI/render setup
- REPL mutable runtime (`regexes`, result cache, shell stack)
- token lifecycle (MREG/OSP/sudo), JWKS caching, auth refresh
- API client construction and lazy initialization
- orchestrator context (`task_id`, `host_ref`, mode inference)
- history persistence and expansion helpers

This works, but coupling is high and hard to test as the feature set grows.

## What We Keep

- lazy client creation (fast startup)
- explicit helper methods for token and orch context workflows
- in-memory REPL/session state that does not leak into persistent config
- resolver-backed config mutations with transactional semantics

## What We Change

- No monolithic `State` object with ad-hoc fields.
- No mutable globals for DSL pipeline/result cache.
- No context/profile stabilization loops.
- No auth/network logic inside config resolution.
- Keep config bootstrap explicit and staged:
  path bootstrap, profile bootstrap, then runtime resolution.

## Python Findings (Source of Truth)

Primary files reviewed:

- `osprov-cli/src/osprov_cli/state.py`
- `osprov-cli/src/osprov_cli/cfg/resolver.py`
- `osprov-cli/src/osprov_cli/app.py`
- `osprov-cli/src/osprov_cli/cli/decorators.py`
- `osprov-cli/src/osprov_cli/repl/manager.py`
- `osprov-cli/src/osprov_cli/core/history.py`

Key observations:

- Loader order is declarative (`cmd/session/env/early_defaults/config/secrets/defaults/theme/late_defaults`).
- Resolver does context stabilization and fixed-point callable interpolation.
- REPL command pipeline state is carried in mutable state fields.
- Auth includes token persistence + refresh + sudo token TTL + JWKS cache.
- Orchestrator has separate "working context" and "last context" semantics.

## Rust Target Model

`osp-cli` keeps a thin `AppState` root, but each concern is a typed sub-state:

- `ConfigState`
  - `ResolvedConfig`
  - strict schema, source/scope metadata
  - explain/introspection APIs
- `UiState`
  - render settings
  - resolved user-facing toggles (`format/mode/color/unicode`)
- `ReplState`
  - prompt mode, history behavior, shell stack
  - no auth/config mutation logic
- `SessionState`
  - in-memory session layer + command-scoped values
  - carries launch defaults that map to config keys (`--json`, `--mode`,
    `--color`, `--ascii`, `-v/-q`, `-d`, `--theme`, `-u`) and REPL
    `config set --session ...` mutations
  - also carries result cache and previous rows
  - dies with process
- `ClientsState`
  - lazy adapters (`ldap`, later `mreg`, `osp`, etc.)
  - depends on ports, not CLI parsing
- `AuthState` (deferred after LDAP MVP)
  - token cache + refresh policy + secure storage adapter
- `OrchState` (deferred)
  - task context, mode inference, last rows

## State Invariants

- `AppState` is a container of sub-states and runtime context.
- `ConfigState` is replaced atomically (whole snapshot), never partially mutated.
- `ConfigState` carries a monotonic `revision`.
- `ClientsState` tracks the config revision it was built against.
  - if config revision changes, clients are invalidated/rebuilt.
- `SessionState` and `ReplState` are the only runtime-mutable state domains.
- There is one in-memory session config layer.
  - Launch-time defaults and REPL session mutations share that same layer.
  - REPL reload rebuilds config from that layer instead of patching live state.
- Command-local flags are not persisted.
  - They are applied ephemerally during dispatch only.

## RuntimeContext Contract

Use one explicit runtime context object across bootstrap, config resolution, and
dispatch:

- `profile_override` (optional)
- `terminal_kind` (`cli` vs `repl`)
- `terminal_env` (`$TERM`, optional)

No subsystem should re-derive these independently.

## Ownership Rules

- `osp-config` owns loading/merging/validation/explain.
- `osp-cli` owns runtime orchestration and sub-state wiring.
- `osp-services` owns business workflows.
- `osp-repl` owns interaction loop and history expansion behavior.
- `osp-ui` only renders values; never mutates state.

## MVP State Scope (Now)

For first LDAP REPL delivery (mocked LDAP, no login):

- keep `ConfigState`, `UiState`, `ReplState`, `ClientsState`
- add `SessionState` for REPL cache/pipeline artifacts
- keep plugin manager state in `ClientsState` (already implemented)
- avoid auth/token state until real LDAP auth/login phase

## Deferred State Scope (After LDAP MVP)

### AuthState

- OSP bearer token selection (sudo token > user token)
- token expiry checks
- refresh strategy and persistence abstraction
- secrets backend bridge (file/env/keyring later)
- auth/token writes go through a `TokenStore`/`SecretsStore` port, not direct
  mutation of config resolver internals

### OrchState

- context setters/getters (`task_id`, `host_ref`)
- last context + last rows cache
- mode explicit vs inferred

## TDD Plan

Start with high-signal contract tests, then fill unit gaps.

### 1) State wiring contracts

- `osp` boots with deterministic `AppState` composition.
- `osp <profile>` switches profile in `ConfigState`.
- REPL startup sees same resolved config as one-shot path.
- `osp <profile> ...` and `osp --profile <profile> ...` resolve equivalent
  profile config semantics.

### 2) Session behavior contracts

- REPL cache does not affect one-shot commands.
- shell/history expansion behavior is deterministic.
- pipeline artifacts are command-local.
- launch flags that map to config keys become session-scoped defaults.
- session `config set` overwrites those defaults in the same in-memory layer.
- per-command `-v/-q/-d` do not leak across commands.

### 3) Config mutation contracts (later)

- session overrides are in-memory only.
- persistent writes route by key class (config vs secrets).
- invalid write rolls back and keeps previous resolved config.

### 4) Auth contracts (later)

- valid token path requires no login prompt.
- expired token triggers refresh path.
- sudo token ttl + invalidation behavior is deterministic.

## Immediate Next Steps

1. Move more REPL mutable artifacts (history/shell stack semantics) behind `SessionState`.
2. Add `ClientsState` config-revision invalidation to real client factories (LDAP/MREG/OSP adapters).
3. Keep auth/orch out of MVP code path; implement as separate modules later.
4. Gate every added state capability behind integration/contract coverage.
