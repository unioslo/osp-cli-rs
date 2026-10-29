# REPL

The REPL runs the same commands as one-shot `osp` and keeps shell scope,
history, the last result, and session configuration between commands.

Use the REPL when you are exploring, iterating, or repeatedly looking at the
same backend data with different pipes and output formats. Use one-shot CLI
commands when you are scripting, automating, or just need one answer.

Examples that use `inventory ...` below are illustrative provider-backed
commands. Replace them with a real plugin command from
`plugins list`.

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

With terminal colour enabled, the intro starts on a clean screen after moving
visible output into terminal scrollback. Ctrl-L clears the REPL the same way
and keeps the current input. Clearing uses the terminal height when a cursor
row is unavailable without a blocking query; it never purges scrollback.

## Intro and help documents

The intro and root `help` compose ordered semantic document blocks. Configure
`repl.intro` and `repl.intro_template.minimal`, `.compact`, or `.full` to author
an intro with Markdown headings, paragraphs, inline markup, and lists. Nested
and loose list items retain their parent text. `{{ help }}` and `{{ overview }}`
insert the root help blocks at that position.

Fenced `osp` blocks contain passive JSON data; other fences and invalid JSON
remain literal text. Repeated fences stay separate and ordered. Templates do
not execute commands. `intro --json` emits a document object; `intro | L 2`
limits its flat rows. Root help uses the same row pipeline and section headings.

## Same Grammar as the CLI

If a command works as a one-shot invocation, the same command text should work
inside the REPL.

```bash
osp plugins list --json -v
```

```text
plugins list --json -v
```

That includes:

- DSL pipes
- `--format` and format shorthands like `--json`
- `--source-record` to select the source record before rendering or pipes
- `-v/-q/-d`

Failures use the same renderer as one-shot commands: `error:`, an optional
`request-id:`, and at most one `hint:`. Failure wording stays stable as verbosity
increases. Plugin messages preserve sanitized text, levels and ordering on
failed commands; known injected secrets are redacted.

Launch `-v`/`-q`/`-d` flags set session defaults. Per-line flags add to those
defaults for one command only. `-v` explains service and session selection,
`-vv` shows exchanges, and `-vvv` adds response/cache evidence. These diagnostics
and verbose field supplements are complete stderr lines above the prompt.
Background notices use the editor's printer to appear above the entire prompt,
including multiline prompts, and wait until input resumes after commands.
`-d` shows host dispatch; `-dd` adds internal host decisions and captured
backtraces. File logging is independent of per-line stderr settings.

Read the error and its hint. `doctor last -vvv` reads retained sanitized evidence
without repeating the failed operation. `doctor last -dd` writes the captured
backtrace to stderr. `doctor` checks local setup without contacting services.
A configured `repl.intro=none` remains disabled at every verbosity level.

For successful results, the REPL also keeps the last replayable output:

- `last`
  - replay the last successful result with its original pipe stages
- `last --raw`
  - show the pre-pipeline payload from that same successful command
- `last --source-record`
  - select the saved source record before replaying the original pipeline
  - combine with `--raw` to skip that pipeline and inspect the full source
  - without a separate source, use the command's normal data and presentation
- `last | state`, `last --json`, `last --raw | P name`
  - use normal output formats and DSL stages without another service request
  - new stages follow the saved pipeline; `--raw` skips that saved pipeline
  - replay keeps the saved result intact, so later `last` calls start from it
  - plain `last` retains the original command's source/display selection
- `doctor last`
  - stays failure-only

## What The REPL Adds

The REPL is useful because it keeps a few things alive across commands:

- shell scope
- history and history expansion
- completion
- the last successful result for local replay and inspection
- session-scoped config overrides
- session context published by native commands and plugins

Context kinds include `account`, `group`, `person`, `host`, `netgroup`,
`contact`, `task`, `provider`, `vm`, `network` and `zone`. Services declare which
they consume and produce; only reads may fill an omitted subject.

Use `context` to inspect current entities: kind, value, service,
identity/profile scope, selecting command, age and last consuming command.
`context set account alice` pins a value. `context clear` removes visible
entries, including persisted copies; `context clear host` removes only hosts.
Normal output flags and DSL pipes work, for example `context --json` or
`context | F kind == 'host'`.

Read arguments declared by commands may be omitted; the host announces the
remembered value and stores the expanded command in history. Writes require an
explicit value or `.`; expansion is echoed before execution. Prompt tokens use
`kind:value`, qualified by service only when kinds clash.
For declared read subjects, an explicit argument wins over context, and context
wins over the command's default value.

Changing profile, effective identity or service endpoint hides foreign values;
switching back restores them. Logout removes the service's values. Most kinds
live only for the REPL process. Services may opt a kind into persistence: those
entries load before the first prompt and are ignored by one-shot commands.
An opted-in shared kind also persists when set by another command or `context set`.
`context` itself is REPL-only.

`context.seed` defaults to `["account", "netgroup"]`; set it to `[]` to disable
seeding. Native integrations may offer defaults through `context_seeds` using
local identity or cached data. The host fills only empty kinds after loading
persisted context, and checks again before commands so a background cache can
arrive later. Selections remain scoped to the active identity and profile.

## Shell Scope

Suggested commands match the current prompt. Inside `orch`, both `task …` and
`orch task …` work unless a host command or alias shadows the relative first
word: use `orch help task` to distinguish it from the host's `help` command.
Suggestions for another root keep their root form; leave the current shell
before running them. JSON follow-ups in the REPL use the same form, while
one-shot output uses `osp …`.

Products choose shellable roots through `repl.shellable_commands`; the library
enables none by default.
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
until you leave it. Entry prints one line explaining `help`, `exit`, and
Ctrl-D; use `help` to list the scoped commands. Ctrl-D on an empty input line
exits OSP from root or a shell.

Inside an active shell, repeating the current root shows help for that shell:

```text
ldap
ldap
```

The second `ldap` is interpreted as `ldap --help`.

Shells do not nest. Typing another shell's root switches to it: inside `orch`,
`nh` leaves the `orch` shell and enters the `nh` shell, and `exit` returns to
root.

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
- history search, bang replay and leading-space privacy; see
  [COMPLETION.md](COMPLETION.md#history) for keys and persistence rules
- paged results: `next` and `prev`, or Shift+→ / Shift+← (also PageDown /
  PageUp) on an empty line; these keys do nothing without a paginated result.
  Typed `next` / `prev` still report when there is no result to continue.
  With text on the line, Shift+arrows move the cursor

Completion does not call remote services while you are typing. It works from
the known command catalog, config vocabulary, and already-available runtime
state.

See [COMPLETION.md](COMPLETION.md).

`history clear PATTERN [--preview] [--force]` deletes matching displayed lines in the current history scope: matching starts at the beginning, `*` and `?` are wildcards, and a trailing `*` is implicit. More than 100 matches require `--force`; `--preview` lists the original history numbers without deleting. Root history includes shell prefixes; inside a shell the prefix is stripped, as in `history list` and Ctrl-R.

`repl.history.record` defaults to `commands`, retaining lines whose first command
word resolves in the current scope even when later arguments or execution fail:
keep fixable attempts, drop junk. `all` records every otherwise eligible line;
`off` disables recording. The existing history enablement switches, privacy rules
and exclusions still apply.

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
config set repl.exit_message 'So long, and thanks for all the fish!'
```

The message is printed for root `exit`, `quit`, or end-of-input at any scope.
Using `exit` or `quit` in a command shell returns to root with a
`Leaving …` message instead.

## Config Writes Inside The REPL

Inside the REPL, `config set` saves to the active profile unless
`config.default-target` or explicit scope/store flags select another
destination. Use `--session` to force a temporary change.
[CONFIG.md](CONFIG.md#repl-config-writes) explains store
and scope selection.

```text
config set ui.format json
config set ui.format json --session
```

The first form saves the setting; the second changes only the current session.

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
- `command.prefixes`
  - `off | interactive | always`; see [COMPLETION.md](COMPLETION.md)
- `repl.hint.command_limit`
  - Maximum names in typing hints, default `3`; `0` hides multiple-match lists.
- `repl.autosuggest`
  - Inline suggestions, default `true`; Ctrl-Right cycles history choices at
    the end of input with the menu closed. Status/help guidance stays enabled.
- `repl.complete_on`
  - List of `flag`, `space`, `after_<n>_letters`; default `["flag"]`.
    See [COMPLETION.md](COMPLETION.md).

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
plugins list --format md
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

## Shortcuts and terminal bells

Products choose named actions and bindings; basic and JSON sessions omit product
shortcuts. See [EMBEDDING.md](EMBEDDING.md#repl-product-hooks) for the host API.

The `last` result cache retains data stages, without repeating output sinks.

Alt+L pages the last staged result, Alt+E opens a read-only temporary copy in
`$VISUAL` / `$EDITOR` / `vi`, and Alt+Y copies it. Each action re-renders retained
structured output without screen width, margin or display limits. The pager
keeps configured colour; the editor has no colour, and clipboard output is also
ASCII. Plain-text commands fall back to retained text. These actions never
execute the command or pipeline again; editor changes are discarded. JSON,
CSV and TSV are printed directly instead of paged. The REPL never auto-pages.
Before
any output is available they show a brief hint. Alt+L and Alt+Y replace the
editor's downcase-word and yank-pop bindings.

Configure `repl.bindings.output.page`, `repl.bindings.output.editor` and
`repl.bindings.output.copy` with F2–F12, Alt+Enter, Alt+L, Alt+E, Alt+Y or `none`.
The output shortcuts are available in interactive sessions; basic and JSON
sessions omit shortcuts as above.

### Terminal bells

`repl.notifications.sound = "off" | "bell"` is the master policy (library default
`off`; products can choose another default). `notifications preview` explicitly
exercises the same local sink without HTTP. It respects the master, rate limit
and machine-output suppression. Native commands request semantic feedback with
`NativeCommandContext::request_bell()`, flushed after foreground output. Workers
may request a best-effort bell through their current `ReplSession`. Background
bells are immediate and subject to the same master policy and rate limit.

BEL is best effort: the local terminal may play a sound, flash or ignore it,
including over SSH and tmux. OSP never runs an audio subprocess or configures
terminal passthrough. At most one BEL is emitted per ten seconds process-wide.
One-shot, basic/non-TTY and JSON output never emit event or preview bells.
Disable sound for this session with
`config set repl.notifications.sound off --session`. Muting never replays audio.

Intro templates and daily tips may use `{{actions}}` to show registered action
labels with their resolved keys; disabled and unavailable actions are omitted.
The root help surface uses the same resolved metadata.

A leading `|` replays the saved answer: `| F status=failed` means
`last | F status=failed` and makes no new request. Separate leading-pipe lines
each start from the saved answer, not the previous replay. Quoted literal pipes
remain ordinary command arguments.

Pasting `osp COMMAND …` with a known root command removes only `osp` and
returns the rest as editable input. Press Enter to execute it. Launcher flags
such as `osp -u USER …` or `osp --prod …` stay visible; these lines are not
normalized because their first word after `osp` is not a command root.

`!! <args>` recalls the last command and appends arguments as editable input.
For example, `!! --force --override cname` prepares a retry; Enter submits it.

`each 'host {}'` runs once per `VALUE` value of the saved result, after its
pipeline. Values are shell-quoted into `{}`; put the placeholder outside quotes
inside the template. More than 50 values requires `--yes`. Commands that change
state require both `--yes` and `--count N`, where N is exactly the number of
values. Native integrations declare read/write effects on their command metadata;
commands without declarations require acknowledgement unless their resolved leaf
is a recognized read command. Templates with unresolved command names require
acknowledgement. Domain `--force` and named overrides do not acknowledge a bulk
write. Existing command confirmations still apply. `--ignore-errors` continues after failures;
a final `N ok, M failed` tally is always printed after execution. Like `source`,
exit or reload stops the batch, and nested source/each commands are refused.

`help --all` lists every visible command path and description as ordinary rows.
Use `help --all | uptime`, `help --all | F command ~ task`, `help --all --json`,
or `help --all | Y` to filter, export, or copy the catalog. Hidden commands
and positional/value completion entries are excluded.

Alt+Enter inserts the command from the most recent structured recovery hint
into the input line. Press Enter to execute, or edit it first. Subcommand typo
corrections preserve the remaining arguments and flags, including commands
that still need an argument. With two or three suggestions, press Alt+Enter
again to cycle in the displayed order while the inserted line is unchanged;
an edit restarts at the first choice. More than three suggestions stay text-only
and clear the previous insertion choices. Commands are formatted for the current shell. Set `repl.bindings.hint.insert` to `Alt+Enter`
(the default), `F2` through `F12`, `Alt+L`, `Alt+E`, `Alt+Y`, or `none`; duplicate bindings are rejected.
Hints without a command do not replace the remembered command. Native commands
produce hints with `NativeCommandContext::run_hint`; `CommandScope::run_hint`
retains recovery commands in errors. No terminal text is scraped.

## Session recording

`recording start FILE` creates a new private (0600) JSON Lines file.
`recording stop` closes it; `recording status` shows the active file. Recording
also stops when the REPL exits and survives configuration or theme reloads.
Existing files are refused. Each submitted line produces a `command` event
with `line`, `output` events with `stream` and plain `text`, and `http` events
with `method`, sanitized `url`, `status` (null for transport failure), and
`duration_ms`. HTTP events reuse command diagnostics, independent of `-v`.
Headers and request/response bodies are never captured. Leading-space lines,
sensitive command arguments, and explicit secret-display commands are omitted,
including their output and HTTP events. Password prompt input is never captured;
known config secrets and sensitive output fields are redacted.

Replay uses ordinary command files, not recording files. Extract commands with
`jq -r 'select(.event == "command") | .line' session.jsonl > commands.osp`,
then run `source commands.osp`. Review the commands before sourcing a recording.
