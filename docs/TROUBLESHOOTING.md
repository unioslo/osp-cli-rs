# Troubleshooting

This file is for the boring first-pass checks that solve most operator
problems.

The pattern is:

1. identify which stage is failing
2. run the smallest diagnostic that explains that stage
3. only then start guessing

## Start Here

These commands usually tell you the most with the least effort:

- `osp -d ...`
- `osp config explain <key>`
- `osp plugins doctor`
- `osp plugins list`
- `osp plugins commands`

Use them before changing config or blaming the terminal.

## Symptom: A Plugin Command Is Missing

Check, in order:

1. is the plugin discoverable?
2. is the plugin healthy?
3. is the command enabled in the active scope?
4. is there a provider conflict?

Useful commands:

```bash
osp plugins list
osp plugins commands
osp plugins doctor
```

If multiple plugins provide the same command, choose one explicitly:

```bash
osp ldap user alice --plugin-provider uio-ldap
```

Or persist a preferred provider:

```bash
osp plugins select-provider ldap uio-ldap
```

## Symptom: A Plugin Command Times Out

Plugin discovery and execution are bounded by
`extensions.plugins.timeout_ms`.

Useful checks:

```bash
osp -d plugins doctor
osp config get extensions.plugins.timeout_ms
```

If the backend is genuinely slow, increase the timeout. If not, treat timeouts
as plugin health problems first, not as rendering problems.

## Symptom: The Wrong Profile Seems Active

If the command looks like it is hitting the wrong environment, make profile
selection explicit and compare behavior:

```bash
osp --profile tsd plugins list
osp config explain profile.default
```

Remember:

- `--profile` wins when present
- otherwise `osp <profile> ...` only acts as profile shorthand if that profile
  is actually known from config/bootstrap state

If you are unsure, prefer `--profile` while debugging so the command line is
unambiguous.

## Symptom: REPL Startup Or Editing Is Bad In A Weak Terminal

Set:

```bash
osp config set repl.input_mode basic --save
```

`basic` is the safest option for limited terminals, odd PTYs, and terminals
with weak interactive support.

If completion looks semantically wrong rather than visually wrong, also check
[COMPLETION.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/COMPLETION.md).

## Symptom: JSON Output Is Polluted

`osp` keeps machine output on `stdout` and diagnostics on `stderr`.

If a script sees invalid JSON:

1. redirect `stderr`
2. check whether a plugin is printing data to the wrong stream
3. retry with `-d`

Example:

```bash
osp ldap user alice --json 2>debug.log
```

If the JSON becomes valid once `stderr` is redirected, the issue is stream
mixing rather than JSON formatting.

## Symptom: A Config Change Did Not Stick

In the REPL, `config set` defaults to session scope. Use `--save` when you
want the change written to persistent config.

```text
config set ui.format json
config set ui.format json --save
```

If you are still confused, inspect the winner directly:

```bash
osp config explain ui.format
```

## Symptom: Themes Look Wrong

Start with:

```bash
osp theme list
osp theme show dracula
osp doctor theme
osp config explain theme.name
```

That usually answers whether the wrong theme won, a custom theme failed to
load, or a custom file overrode a builtin.

## More Help

- REPL usage: [REPL.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/REPL.md)
- Completion and history: [COMPLETION.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/COMPLETION.md)
- Invocation flags: [FORMATTING.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/FORMATTING.md)
- Config: [CONFIG.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/CONFIG.md)
- Plugins: [USING_PLUGINS.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/USING_PLUGINS.md)
