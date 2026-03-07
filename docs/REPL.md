# REPL Design

The REPL is a thin loop that reuses the one-shot command execution path.


## MVP scope

For the first working version, the REPL only needs to support:

- `ldap user <uid>`
- `ldap netgroup <name>`
- DSL pipes after LDAP output

No login flow is required in MVP.

## Core behavior

- Start with a profile-resolved prompt.
- Parse each REPL line with clap, then dispatch through the same command enums
  as one-shot mode (`Commands`).
- Support history and basic completion.
- Support the same DSL pipe syntax as one-shot commands.

## Command Parsing Contract

- REPL lines are split with shell-compatible tokenization.
- Tokens are parsed by a dedicated clap parser (`ReplCli`) that accepts the
  same command tree as one-shot mode.
- No ad-hoc `args[0] == "..."` parsing for built-ins.
- Unknown top-level commands route through clap external subcommand handling,
  then to plugin dispatch.
- Help/version parse output from clap is rendered directly in REPL.

## DSL Capability Contract

- DSL support is declared per parsed command variant, not by string matching.
- `plugins list|commands|doctor` and `theme list|show` support DSL pipelines.
- mutation commands like `plugins enable|disable` and `theme use` do not
  support DSL stages.
- plugin external commands support DSL by default because they return row data.

## Prompt Config

Prompt appearance is now config-seeded (with CLI color/unicode/mode overrides
still respected):

- `ui.presentation`
- `repl.prompt`: multiline template (`{user}`, `{domain}`, `{profile}`,
  `{context}`, `{indicator}`)
- `repl.simple_prompt`: one-line prompt mode (`<profile> >`)
- `repl.shell_indicator`: template for shell stack display (`{shell}`)
- `repl.intro`: show/hide startup intro banner
- `repl.intro.style`: `full | minimal | none`
- `ui.help.layout`: `full | compact | minimal`
- `ui.messages.layout`: `grouped | minimal`
- `ui.chrome.frame`
- `color.prompt.text`: optional explicit style spec for prompt text
- `color.prompt.command`: optional explicit style spec for profile/command part

Current preset behavior:

- `expressive` keeps the multiline prompt and full intro
- `compact` prefers the simple prompt and minimal intro
- `austere` prefers the simple prompt, minimal intro, and minimal help/message
  density

## Scope

The REPL now supports:

- prompt and intro profiles
- shell-scoped history
- command/flag completion
- invocation-local flags such as `--json`, `--mode`, `-q`, and `--cache`
- inline help/error recovery for partial builtin commands

Config mutation behavior:

- `config set` in REPL defaults to session scope
- prompt/theme/presentation changes rebuild the REPL state on the next cycle
- use `--save` when you want the change persisted

## Completion integration

Completion should come from a command tree built at startup. It should not
hit network services. Dynamic data is injected from state.

For the full completion plan and crate choices, see docs/COMPLETION.md.

## Line editor choice

We should use a line editor that supports:

- Ctrl-R reverse search
- Multi-column completion menus
- Custom prompts with left and right sections
- Syntax highlighting for valid commands

The recommended default is `reedline`. It maps well to prompt_toolkit
capabilities in the current Python implementation.
