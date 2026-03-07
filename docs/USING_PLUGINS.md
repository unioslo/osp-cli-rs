# Using Plugins

Most domain commands in `osp` come from plugins.

## Inspect Plugins

Useful commands:

- `osp plugins list`
- `osp plugins commands`
- `osp plugins doctor`
- `osp plugins refresh`

Use `plugins doctor` first when a command is missing or behaving unexpectedly.

## Enable and Disable Plugins

```bash
osp plugins enable uio-ldap
osp plugins disable uio-ldap
```

## Discovery Order

`osp` looks for plugins in this order:

1. `--plugin-dir <dir>`
2. `OSP_PLUGIN_PATH`
3. bundled plugin directory
4. `~/.config/osp/plugins`
5. `PATH` (`osp-*` executables)

## Provider Conflicts

If exactly one active plugin provides a command, `osp` uses it automatically.

If multiple active plugins provide the same command, `osp` does not guess.
Choose a provider for one invocation:

```bash
osp ldap user alice --plugin-provider uio-ldap
```

Or store a preferred provider:

```bash
osp plugins select-provider ldap uio-ldap
osp plugins clear-provider ldap
```

`osp plugins commands` and REPL help/completion show unresolved conflicts
instead of inventing a merged command grammar.

## REPL Usage

The same provider-selection rules apply in the REPL:

```text
ldap user alice --plugin-provider uio-ldap
```

Inside a scoped shell:

```text
user alice --plugin-provider uio-ldap
```

## Plugin Timeouts

Plugin discovery and execution are bounded by
`extensions.plugins.timeout_ms`. Default: `10000`.

If a plugin times out:

1. run `osp plugins doctor`
2. retry with `-d`
3. increase `extensions.plugins.timeout_ms` if the backend is expected to be slow

## More Plugin Docs

- Authoring guide: [WRITING_PLUGINS.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/WRITING_PLUGINS.md)
- Packaging and manifests: [PLUGIN_PACKAGING.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/PLUGIN_PACKAGING.md)
- Protocol details: [PLUGIN_PROTOCOL.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/PLUGIN_PROTOCOL.md)
