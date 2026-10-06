# REPL

The REPL runs the same commands as one-shot `osp` and keeps shell scope,
history, the last result, and session configuration between commands.

Use the REPL when you are exploring, iterating, or repeatedly looking at the
same backend data with different pipes and output formats. Use one-shot CLI
commands when you are scripting, automating, or just need one answer.

Examples that use `inventory ...` below are illustrative provider-backed
commands. Replace them with a real plugin command from
`plugins commands`.

## How a line is processed

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

## Start the REPL

```bash
osp
```

## Same Grammar as the CLI

If a command works as a one-shot invocation, the same command text should work
inside the REPL.

```bash
osp plugins commands --json -v
```

```text
plugins commands --json -v
```

That includes:

- DSL pipes
- `--format` and format shorthands like `--json`
- `-v/-q/-d`
- `--plugin-provider`

For failures, the same detail ladder applies inside the REPL:

- default
  - terse actionable summary
- `-v`
  - summary plus cause/context chain
- `-vv`
  - rich diagnostic report
- `-vvv`
  - rich diagnostic report plus stack backtrace

`-d/-dd/-ddd` are still about debug logs and timing, not the failure render
itself.

Practical failure workflow:

1. run the command normally
2. if it fails, read the terse failure
3. rerun the command with `-v`, `-vv`, or `-vvv`, or inspect the recorded
   failure with `doctor last -v`, `doctor last -vv`, or `doctor last -vvv`

For successful results, the REPL also keeps the last replayable output:

- `last`
  - replay the last successful result with its original pipe stages
- `last --raw`
  - show the pre-pipeline payload from that same successful command
- `last | state`, `last --json`, `last --raw | P name`
  - use normal output formats and DSL stages without another service request
  - new stages follow the saved pipeline; `--raw` skips that saved pipeline
  - replay keeps the saved result intact, so later `last` calls start from it
- `doctor last`
  - stays failure-only

## What The REPL Adds

The REPL is useful because it keeps a few things alive across commands:

- shell scope
- history and history expansion
- completion
- the last successful result for local replay and inspection
- session-scoped config overrides

## Shell Scope

`repl.shellable_commands` selects eligible
top-level roots; its defaults are `nh`, `mreg`, `ldap`, `vm`, and `orch`.
Built-in namespaces such as `plugins`, `config`, `theme`, and `help` do not
become shells.

Most plain upstream installs will not expose those domain roots, so you can
ignore shell scope until your downstream distribution provides one. When it
does, shell entry is shell-first:

```text
ldap
user oistes
exit
```

Typing the bare root enters that shell and makes later lines inherit the root
until you leave it.

Inside an active shell, repeating the current root shows help for that shell:

```text
ldap
ldap
```

The second `ldap` is interpreted as `ldap --help`.

The hidden `cd <root>` command handles the rare case where a nested shell has
the same name as its parent.

Shell controls such as `exit`, `quit`, and bare `help` stay REPL-owned. They
manage the shell rather than dispatching a normal command.

Host commands such as `doctor last -v`, `config`, and `history` keep their root
meaning inside integration shells. Scoped completion includes the same global
commands that dispatch accepts; you do not need to leave `[orch]` to inspect a
failure.

## History And Completion

The REPL provides:

- shell-like tokenization
- command and flag completion
- scoped completion inside shells
- history navigation
- `!!` repeats the last accepted command, including one that failed during
  execution; `sudo !!` repeats it with elevation. Tab expands these into the
  editor without running; Enter expands and runs them. Unknown commands and
  invalid syntax are excluded from saved history.
- saved-history expansion such as `!123`, `!-2`, and `!prefix`

Completion does not call remote services while you are typing. It works from
the known command catalog, config vocabulary, and already-available runtime
state.

See [COMPLETION.md](COMPLETION.md).

## Command Aliases

Use the `alias` wrapper for ordinary alias work instead of spelling config
keys by hand:

```text
alias add lsng 'ldap netgroup ${1} --value | P members'
alias list --sources
alias remove lsng
```

Templates are checked by the same placeholder, command, and DSL parsers used
when an alias runs. Positional placeholders such as `${1}` and `${@}` remain
unresolved until invocation.

Aliases follow config scope and storage rules. Without a configured
`config.default-target` or explicit scope/store flags, a one-shot CLI write
persists, while a REPL write lasts for the current session. `--permanent`
requests persistence; `--profile`, `--global`, and `--terminal` select the same
scopes as `config set` and `config unset`.

The root REPL is silent on exit by default. Set `repl.exit_message` when a
product or profile wants a sign-off:

```text
config set repl.exit_message 'So long, and thanks for all the fish!' --permanent
```

The message is printed for root `exit`, `quit`, or end-of-input. Leaving a
nested command shell keeps its existing `Leaving … shell` message instead.

## Config Writes Inside The REPL

Inside the REPL, `config set` defaults to session storage unless
`config.default-target` or explicit scope/store flags select another
destination. Use `--session` to force a temporary change and `--permanent` to
request persistence. [CONFIG.md](CONFIG.md#repl-config-writes) explains store
and scope selection.

```text
config set ui.format json
config set ui.format json --permanent
```

Use the first form when you want to experiment. Use `--permanent` when you have
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

Run a batch in the current authenticated session:

```text
source changes.osp
source --ignore-errors follow-up.osp
```

Command files contain one command per line. Blank lines and lines beginning
with `#` are ignored. By default execution stops at the first failure and
reports its file and line number; `--ignore-errors` processes the rest.

Do a short built-in investigation:

```text
plugins list
plugins commands --format md
help config
```

Change presentation temporarily for the current session:

```text
config set ui.presentation compact
config set repl.simple_prompt true
```

Inspect the most recent result, then keep slicing it locally:

```text
last
last --raw
last --format json
```

## When Not To Use The REPL

Prefer one-shot commands when:

- you are scripting or piping into other tools
- you need exact reproducible output in CI or shell scripts
- the command is a one-off and session state adds no value

In those cases, use ordinary CLI commands with explicit render flags such as:

```bash
osp --format json --render-mode plain plugins list
```
