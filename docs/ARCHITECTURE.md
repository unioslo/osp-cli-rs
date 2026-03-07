# Architecture Contract

This is a hard crate boundary contract.
It is enforced by Cargo workspace dependencies, not just conventions.

## Workspace Layout

```text
crates/
  osp-core/
  osp-config/
  osp-dsl/
  osp-ports/
  osp-api/
  osp-services/
  osp-ui/
  osp-repl/
  osp-cli/
```

## Crate Responsibilities

- `osp-core`
  - Shared primitive types (`Row`) and output enums.
  - No project-internal dependencies.
- `osp-config`
  - Bootstrap + runtime config model and loading.
  - Owns path bootstrap, profile bootstrap, runtime resolution, and explain.
- `osp-dsl`
  - DSL tokenizer/parser/stage execution (`P`, `F`, `V`).
- `osp-ports`
  - Domain-facing traits and contracts (e.g. `LdapDirectory`).
  - Shared LDAP filter/attribute semantics.
- `osp-api`
  - Concrete adapters implementing ports (MVP: mocked LDAP backend).
- `osp-services`
  - Use-case orchestration, command parsing, command execution.
  - Bridges ports + DSL.
- `osp-ui`
  - Rendering (`json`, `table`, `mreg`, `value`) and color/unicode behavior.
- `osp-repl`
  - Interactive loop, history expansion, completion wiring.
- `osp-cli`
  - Binary entrypoint, clap tree, one-shot dispatch, REPL bootstrapping.
  - Owns executable plugin discovery/dispatch and plugin state commands.
  - Owns command ownership policy: domain verbs are plugin-owned.
  - Internally split by concern under `crates/osp-cli/src/app/` and
    `crates/osp-cli/src/repl/` so the top-level entrypoints stay orchestration-only.

## Allowed Crate Dependencies

- `osp-cli` -> `osp-repl`, `osp-ui`, `osp-dsl`, `osp-core`
- `osp-repl` -> no `osp-*` dependencies
- `osp-services` -> `osp-ports`, `osp-dsl`, `osp-config`, `osp-core`
- `osp-api` -> `osp-ports`, `osp-config`, `osp-core`
- `osp-ui` -> `osp-core`
- `osp-dsl` -> `osp-core`
- `osp-config` -> `osp-core`
- `osp-ports` -> `osp-core`
- `osp-core` -> std + external crates only

## Forbidden Dependencies

- `osp-core` depending on any `osp-*` crate.
- `osp-dsl` depending on `osp-services`, `osp-api`, `osp-ui`, `osp-repl`, `osp-cli`.
- `osp-ports` depending on `osp-api`, `osp-services`, `osp-repl`, `osp-cli`, `osp-ui`, `osp-dsl`.
- `osp-ui` depending on `osp-api`, `osp-services`, `osp-repl`, `osp-cli`, `osp-dsl`, `osp-ports`.
- `osp-api` depending on `osp-services`, `osp-repl`, `osp-cli`, `osp-ui`, `osp-dsl`.
- `osp-services` depending on `osp-api`, `osp-repl`, `osp-cli`, `osp-ui`.
- `osp-repl` depending on `osp-api`, `osp-cli`, `osp-dsl`, `osp-config`.

## Command Ownership

- Backbone (`osp-cli`) owns:
  - plugin management (`osp plugins ...`)
  - output/render toggles
  - REPL loop and help/completion integration
- Plugins own domain commands:
  - `ldap`, `mreg`, and other provider/domain verbs
- Result:
  - `osp ldap ...` without plugin now fails fast with `no plugin provides command: ldap`.

## Data Ownership Rules

- `osp_core::row::Row` is the canonical row payload type across crates.
- Ports define trait contracts; adapters implement them.
- Rendering never owns business semantics.
- CLI/repl never implements LDAP semantics directly.
- `osp-cli` runtime state is split into:
  `RuntimeContext`, `ConfigState`, `UiState`, `ReplState`, `SessionState`,
  `ClientsState`.

## Current osp-cli Flow

For the current command path, start at `crates/osp-cli/src/app.rs`.

- Startup/bootstrap details live in `crates/osp-cli/src/app/bootstrap.rs`.
- One-shot dispatch planning lives in `crates/osp-cli/src/app/dispatch.rs`.
- External command execution lives in `crates/osp-cli/src/app/external.rs`.
- Command output rendering lives in `crates/osp-cli/src/app/command_output.rs`.
- REPL rebuild/reload lives in `crates/osp-cli/src/app/repl_lifecycle.rs`.

Config startup is intentionally staged:

1. bootstrap discovers config/secrets paths
2. loaders read the configured layers
3. bootstrap selects the active profile
4. runtime resolution produces the final config snapshot

For the interactive path, start at `crates/osp-cli/src/repl/mod.rs`.

- REPL loop lifecycle lives in `crates/osp-cli/src/repl/lifecycle.rs`.
- REPL line dispatch lives in `crates/osp-cli/src/repl/dispatch.rs`.
- REPL completion tree shaping lives in `crates/osp-cli/src/repl/completion.rs`.

See `docs/COMMAND_FLOW.md` for a short "where do I read next?" guide.

## PR Gate Checklist

- [ ] Any new crate dependency follows this file.
- [ ] No cross-layer shortcut imports added.
- [ ] User-visible behavior has a contract test.
- [ ] Cross-crate flows have integration coverage.
