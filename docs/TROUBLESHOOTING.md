# Troubleshooting

## Start Here

These commands are the fastest way to understand what `osp` is doing:

- `osp -d ...`
- `osp config explain <key>`
- `osp plugins doctor`
- `osp plugins list`
- `osp plugins commands`

## A Plugin Command Is Missing

Check:

1. the plugin is discoverable
2. the plugin is healthy
3. the plugin is enabled
4. there is not a provider conflict for the command

If multiple plugins provide the same command, choose one explicitly:

```bash
osp ldap user alice --plugin-provider uio-ldap
```

## A Plugin Command Times Out

Plugin discovery and execution use `extensions.plugins.timeout_ms`.

Useful checks:

```bash
osp -d plugins doctor
osp config get extensions.plugins.timeout_ms
```

## REPL Startup Is Bad in a Weak Terminal

Set:

```bash
osp config set repl.input_mode basic --save
```

`basic` is the safest option for limited terminals and unusual PTYs.

## JSON Output Is Polluted

`osp` keeps machine output on `stdout` and diagnostics on `stderr`.

If a script sees invalid JSON:

1. redirect `stderr`
2. check whether a plugin is printing data to the wrong stream
3. retry with `-d`

Example:

```bash
osp ldap user alice --json 2>debug.log
```

## A Config Change Did Not Stick

In the REPL, `config set` defaults to session scope. Use `--save` when you
want the change written to persistent config.

```text
config set ui.format json
config set ui.format json --save
```

## More Help

- REPL usage: [REPL.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/REPL.md)
- Invocation flags: [FORMATTING.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/FORMATTING.md)
- Config: [CONFIG.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/CONFIG.md)
- Plugins: [USING_PLUGINS.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/USING_PLUGINS.md)
