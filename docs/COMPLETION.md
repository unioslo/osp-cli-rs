# Completion and History

This document covers the REPL features that help you type less and repeat
yourself less.

The short version:

- completion helps you discover what can be typed next
- history helps you reuse what you already typed

Both are local features. They should feel fast and predictable, not magical.

## Broad-Strokes Model

Completion works from already-known local information:

- the visible command catalog
- built-in command/flag metadata
- plugin-provided command metadata that has already been discovered
- config-key metadata
- the current shell scope
- the current cursor position and tokens already on the line

It does not call remote systems while you are pressing tab.

## What Completion Can Suggest

The REPL completes:

- commands and subcommands
- flags
- flag values
- `config set` keys
- scoped commands inside a supported REPL shell, when your install exposes one
- DSL stages after `|`
- path-like values when a command or plugin marks them as path inputs

Examples:

```text
plugins <TAB>
config set <TAB>
theme use <TAB>
plugins commands | <TAB>
```

That is the important contract: completion should help you finish real command
shapes, not just top-level words.

## Context-Aware Suggestions

Completion uses the current command context, including flags already present on
the line.

That matters for things like:

- provider-specific values
- path-like flag values
- DSL suggestions after a pipeline marker
- shell-scoped commands after entering a supported shellable root

Built-in namespaces like `plugins`, `config`, and `theme` do not become
shells. Shell controls like `exit` and `quit` remain REPL-owned.

## Invocation Flags

REPL completion also knows about the invocation-level flags shared with the
CLI, such as:

- `--format`
- `--guide`
- `--json`
- `--verbose`
- `--quiet`
- `--debug`
- `--plugin-provider`
- `--cache`

Parsing also accepts short forms like `-v/-q/-d`, but completion suggestions
use the long names.

## History

The REPL keeps history for two different jobs:

- navigation through previous commands
- history expansion when you want to replay or adapt an earlier command

Supported expansions:

- `!!`
  - last command
- `!-N`
  - Nth previous command
- `!123`
  - absolute history entry
- `!prefix`
  - most recent command starting with that prefix
- `!?text`
  - most recent command containing that text

Examples:

```text
!! 
!plugins
!-2
```

Persisted history intentionally skips help-like and bang commands such as
`help`, `history list`, `--help`, and `!` expansions. That is deliberate; it
keeps durable history focused on commands you are likely to rerun.

## Scoped History

History is shell-aware. That makes repeated work inside a supported shellable
root more predictable because the shell prefix stays part of the command
context.

If your install never exposes shellable roots, you can ignore this section.

## If Completion Looks Wrong

Start with the boring checks first:

1. confirm the command exists in the current catalog:
   `plugins commands`
2. confirm the plugin/provider state is healthy:
   `plugins doctor`
3. if there is a provider conflict, choose one explicitly with
   `--plugin-provider`
4. if the issue is on `config set`, confirm you are using a real config key

Completion problems are often catalog problems, not editor problems.

If the line looks visually wrong rather than semantically wrong, that is more
likely a REPL/editor presentation issue than a completion-logic issue. See
[REPL.md](REPL.md).
