# REPL

The REPL reuses the same command execution path as one-shot `osp` commands.
The main difference is that shell state and session defaults stay alive between
commands.

## Start the REPL

```bash
osp
```

## Same Command Syntax as the CLI

The same invocation-local flags work in both places:

```bash
osp ldap user alice --json -v
```

```text
ldap user alice --json -v
```

This includes:

- DSL pipes
- `--format` and `--json`
- `-v/-q/-d`
- `--plugin-provider`
- `--cache`

## Shell Scope

The REPL can keep a shell scope so a top-level command becomes implicit.

Example:

```text
ldap
user alice --json
```

That behaves like:

```text
ldap user alice --json
```

Shell controls such as `exit`, `quit`, and bare `help` stay REPL-owned.

## History and Completion

The REPL provides:

- shell-like tokenization
- command and flag completion
- scoped completion inside shells
- history navigation
- history expansion such as `!!`, `!123`, `!-2`, and `!prefix`

Completion does not call remote services while you are typing. Dynamic hints
come from already-known runtime state.

See [COMPLETION.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/COMPLETION.md).

## REPL Cache

`--cache` is REPL-only. It reuses a successful external command result so you
can rerun different pipes or output formats without hitting the same backend
again.

```text
ldap user alice --cache | P uid mail
ldap user alice --cache | P uid
```

The second command reuses the cached plugin response and reapplies the current
pipeline and rendering.

## Prompt and Intro

Useful REPL config keys:

- `repl.prompt`
- `repl.simple_prompt`
- `repl.shell_indicator`
- `repl.intro`
- `repl.intro.style`
- `repl.input_mode`

Presentation presets also affect the REPL:

- `expressive`
- `compact`
- `austere`

## Config Writes in the REPL

`config set` defaults to session scope in the REPL.

Use `--save` if you want the change persisted:

```text
config set ui.format json
config set ui.format json --save
```

Theme, presentation, and prompt-related changes rebuild the REPL on the next
cycle so the new state becomes visible immediately.

## Input Modes

`repl.input_mode` controls the line editor behavior:

- `auto`
- `interactive`
- `basic`

If the REPL feels unreliable in a weak terminal, `basic` is the first setting
to try.
