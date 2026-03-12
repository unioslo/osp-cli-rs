# REPL

This document is about why the REPL is useful in practice.

The short version is: the REPL does not invent a second command system. It
reuses the same command path as one-shot `osp`, but keeps shell scope, history,
cache, and session config alive between commands.

Use the REPL when you are exploring, iterating, or repeatedly looking at the
same backend data with different pipes and output formats. Use one-shot CLI
commands when you are scripting, automating, or just need one answer.

## Broad-Strokes Flow

```text
typed line
  ↓
shell scope + one-shot flags + alias/DSL parsing
  ↓
same command execution path as one-shot `osp`
  ↓
optional DSL pipeline
  ↓
render output
  ↓
keep session state for the next prompt
```

The important part is the last step. The command path is shared; the session is
what changes.

## Start the REPL

```bash
osp
```

## Same Grammar as the CLI

If a command works as a one-shot invocation, the same command text should work
inside the REPL.

```bash
osp ldap user alice --json -v
```

```text
ldap user alice --json -v
```

That includes:

- DSL pipes
- `--format` and legacy format shorthands like `--json`
- `-v/-q/-d`
- `--plugin-provider`
- `--cache` inside the REPL

This is the main promise of the REPL: interactive shell on top, same command
language underneath.

## What The REPL Adds

The REPL is useful because it keeps a few things alive across commands:

- shell scope
- history and history expansion
- completion
- cached command results
- session-scoped config overrides

That is the whole point. If you do not need those, a one-shot command is
simpler.

## Shell Scope

Shell scope lets a top-level command namespace stay implicit while you work.

Example:

```text
ldap
user alice --json
group ops
```

That behaves like:

```text
ldap user alice --json
ldap group ops
```

Shell controls such as `exit`, `quit`, and bare `help` stay REPL-owned. They
manage the shell rather than dispatching a normal command.

This is useful when you are spending ten minutes in one namespace and do not
want to keep retyping the same prefix.

## Cache And Repeated Inspection

`--cache` is REPL-only. It reuses a successful command result from the current
session so you can keep changing pipes or output format without hitting the
same backend again.

```text
ldap user alice --cache | P uid mail
ldap user alice --cache | P uid
ldap user alice --cache --format json
```

The second and third commands reuse the same command result and only re-run the
local transform and render path.

This is the highest-value REPL feature when you are exploring one dataset and
want to ask several small questions about it.

## History And Completion

The REPL provides:

- shell-like tokenization
- command and flag completion
- scoped completion inside shells
- history navigation
- history expansion such as `!!`, `!123`, `!-2`, and `!prefix`

Completion does not call remote services while you are typing. It works from
the known command catalog, config vocabulary, and already-available runtime
state.

See [COMPLETION.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/COMPLETION.md).

## Config Writes Inside The REPL

Inside the REPL, `config set` defaults to session scope. That means the change
affects the current session immediately, but is not written to disk unless you
ask for it.

```text
config set ui.format json
config set ui.format json --save
```

Use the first form when you want to experiment. Use `--save` when you have
decided the setting should become part of your stored config.

Theme, presentation, and prompt-related changes rebuild the REPL on the next
cycle so the new state becomes visible immediately.

## Prompt, Intro, And Input Mode

Useful REPL config keys:

- `repl.prompt`
- `repl.simple_prompt`
- `repl.shell_indicator`
- `repl.intro`
  - `none | minimal | compact | full`
- `repl.input_mode`

Presentation presets also affect the REPL:

- `expressive`
- `compact`
- `austere`

Roughly:

- `repl.simple_prompt` controls prompt density
- `repl.intro` controls how much startup/help material appears
- `repl.input_mode` controls how ambitious the line editor should be

If the REPL feels unreliable in a weak terminal, `basic` is the first setting
to try for `repl.input_mode`.

## Practical Recipes

Stay in one namespace for a short investigation:

```text
plugins
list
commands --format md
```

Change presentation temporarily for the current session:

```text
config set ui.presentation compact
config set repl.simple_prompt true
```

Do one external fetch, then keep slicing it locally:

```text
ldap user alice --cache | P uid mail
ldap user alice --cache | P uid
ldap user alice --cache --format json
```

## When Not To Use The REPL

Do not use the REPL just because it exists.

Prefer one-shot commands when:

- you are scripting or piping into other tools
- you need exact reproducible output in CI or shell scripts
- the command is a one-off and session state adds no value

In those cases, use ordinary CLI commands with explicit render flags such as:

```bash
osp --format json --mode plain plugins list
```
