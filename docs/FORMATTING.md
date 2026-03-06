# Command Formatting Hooks (Rust Plan)

This document translates `with_formatting` and related hooks from osprov-cli
into a Rust design. The goal is to keep commands pure while centralizing all
output concerns.

## How `with_formatting` Works Today (Python)

The decorator:

- Adds flags: `--format`, `--mode`, legacy `--json/--table/--value/--md`,
  and `--rich/--plain`.
- Supports `--cache` in REPL.
- Resolves defaults from config (`ui.format`, `ui.mode`).
- Executes the command, then calls `state.ui.format_output(...)`.
- Applies the DSL pipeline.
- Supports clipboard copy via DSL `| Y`.

This is effectively “middleware” around every command.

## Rust Approach: Central Dispatcher

Instead of a decorator per command, we centralize formatting in the command
dispatcher. Commands return data; the dispatcher handles formatting.

### Command result type

```
struct CommandResult {
    data: serde_json::Value,
    format_hint: Option<OutputFormat>,
    allow_cache: bool,
}
```

### Dispatcher flow

1. Parse global formatting flags.
2. Run the command.
3. Apply DSL pipeline to `data`.
4. Choose formatter (`auto` or explicit).
5. Render using plain or rich renderer.
6. Copy if requested or `wants_copy` is set.

## Output Flags (Rust CLI)

We keep the same user-facing flags:

- `--format {json,table,mreg,value,md,auto}`
- `--mode {plain,rich,auto}`
- `--color {auto,always,never}` + `--no-color`
- `--unicode {auto,always,never}` + `--ascii`
- `--copy`

Legacy flags (`--json`, `--table`, `--value`, `--md`, `--rich`, `--plain`) can
be supported temporarily, but should not be advertised.

## Current Session Semantics

The current Rust CLI uses one in-memory session layer for formatting defaults:

- launch `osp --json`, `osp --mode plain`, `osp --color never`, `osp --ascii`
  seed session-scoped config values
- REPL `config set --session ui.format ...` writes into that same layer
- REPL reload rebuilds from that layer
- command-local formatting flags stay ephemeral and do not persist

This means config introspection may show launch formatting flags as
`source=session`, which is intentional.

## Caching (REPL)

In the REPL, cache can be applied at the dispatcher level:

- Hash `(command + args + pipeline)` into a cache key.
- Store `CommandResult` for reuse.
- Cache should be opt-in per command (`allow_cache`).

## Error Handling

Formatting errors should be considered command failures:

- Unknown format -> user error.
- Renderer failure -> internal error.
- Clipboard copy failure -> warn but still print output.

## Tests to Add

- Flag precedence and conflicts (`--format` vs legacy flags).
- `--mode plain` forces plain rendering.
- `--no-color` disables colors regardless of renderer.
- DSL pipeline is applied before formatting.
- `| Y` triggers clipboard copy behavior.
