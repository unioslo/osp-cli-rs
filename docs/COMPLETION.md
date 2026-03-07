# Completion and History

This document describes the user-visible completion and history behavior in the
REPL.

## Completion

The REPL completes:

- commands and subcommands
- flags
- flag values
- scoped commands inside a shell
- DSL stages after `|`

Completion does not make network calls while you are typing. It uses known
command metadata and runtime hints that are already available locally.

## Context-Aware Suggestions

Completion uses the current command context, including flags already present on
the line.

That matters for things like:

- provider-specific flags
- provider-specific values
- path-like flag values
- DSL suggestions after a pipeline marker

## Invocation Flags

REPL completion knows about the same invocation-level flags as the CLI, such as:

- `--format`
- `--json`
- `-v/-q/-d`
- `--plugin-provider`
- `--cache`

Shell controls like `exit` and `quit` remain REPL-owned and are not treated as
normal commands for completion.

## History

The REPL keeps command history for navigation and for history expansion.

Supported expansions:

- `!!` for the last command
- `!-N` for the Nth previous command
- `!123` for an absolute history entry
- `!prefix` for the most recent command with that prefix

## Scoped History

History is aware of shell scope, which makes repeated work inside a command
shell more predictable.

## If Completion Looks Wrong

Start with:

- `help`
- `plugins commands`
- `plugins doctor`

If the issue is specific to a plugin conflict, pick a provider explicitly with
`--plugin-provider`.
