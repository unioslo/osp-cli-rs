# Command Flow Guide

This is the short guide for "where do commands actually go?" in `osp-cli-rust`.

The important thing is that the entrypoints should read like tables of contents.
Detailed work lives in nearby helper modules, not in a new crate.

## CLI Entry Point

Start in `crates/osp-cli/src/app.rs`.

`run()` does three high-level things:

1. normalize CLI input and resolve enough config to understand the runtime
2. build `AppState` plus the dispatch decision
3. hand off to REPL, builtin command execution, or external/plugin execution

If you need more detail, the next files are:

- `crates/osp-cli/src/app/bootstrap.rs`
  - runtime config requests
  - session-layer bootstrap
  - runtime-context and `AppState` construction
  - shared startup/rebuild verbosity helpers
- `crates/osp-cli/src/app/dispatch.rs`
  - `RunAction`
  - profile-prefix parsing (`osp <profile> <command>`)
  - visibility checks
- `crates/osp-cli/src/app/external.rs`
  - alias expansion
  - inline builtin parsing
  - builtin-vs-plugin branching
  - plugin response handling for one-shot external commands
- `crates/osp-cli/src/app/command_output.rs`
  - output rendering
  - message emission
  - clipboard copy handling

## REPL Entry Point

Start in `crates/osp-cli/src/repl/mod.rs`.

`run_plugin_repl()` is intentionally a boring loop:

1. prepare one cycle
2. render shell chrome
3. run the line editor
4. apply the result

If you need more detail, the next files are:

- `crates/osp-cli/src/repl/lifecycle.rs`
  - prompt/help/completion preparation for one cycle
  - loop state transitions
- `crates/osp-cli/src/repl/dispatch.rs`
  - bang/history shortcuts
  - shell enter/exit shortcuts
  - REPL command parsing and execution
  - final continue/restart decision
- `crates/osp-cli/src/repl/completion.rs`
  - REPL-scoped completion tree shaping
  - shell-root completion behavior

## Rebuild Path

Profile changes and explicit reloads rebuild runtime state through
`crates/osp-cli/src/app/repl_lifecycle.rs`.

That file owns:

- what session data survives a rebuild
- how config/theme/plugin state is reconstructed
- how rebuilt state is reattached to the live REPL session

## Shared vs REPL-only

CLI one-shot execution and REPL line execution now share the same core
"turn plugin output into renderable output" path.

Shared pieces:

- plugin dispatch context construction
- plugin response decoding
- plugin message extraction
- DSL pipeline application
- format-hint handling after pipeline stages

Those shared pieces live under `crates/osp-cli/src/app/command_output.rs` and
the nearby app helpers.

REPL-only pieces stay in `crates/osp-cli/src/repl/`:

- bang/history expansion
- shell scope enter/leave behavior
- REPL help shortcuts
- restart/reload decisions after config/theme changes
- line-editor and completion lifecycle

That split is intentional: command execution should be shared where possible,
while interactive shell behavior stays local to the REPL code.

## If You Are New

Use this reading order:

1. `crates/osp-cli/src/app.rs`
2. `crates/osp-cli/src/app/dispatch.rs`
3. `crates/osp-cli/src/app/bootstrap.rs`
4. `crates/osp-cli/src/app/external.rs`
5. `crates/osp-cli/src/repl/mod.rs`
6. `crates/osp-cli/src/repl/dispatch.rs`

That path keeps the high-level flow visible before you drop into the details.
