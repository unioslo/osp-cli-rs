# Using Plugins

Most domain commands in `osp` come from plugins.

## Inspect Plugins

Useful commands:

- `osp plugins list`
- `osp plugins commands`
- `osp plugins doctor`
- `osp plugins refresh`

Use `plugins doctor` first when a command is missing or behaving unexpectedly.

## Enable and Disable Commands

```bash
osp plugins enable ldap
osp plugins disable ldap
osp plugins clear-state ldap
```

These commands persist command routing in the scoped config file. `enable` and
`disable` set `state = "enabled" | "disabled"`, and `clear-state` removes the
explicit override so the command falls back to plugin defaults.

Example:

```toml
[profile.default.plugins.ldap]
state = "enabled"
provider = "uio-ldap"
```

More specific terminal and profile scopes override less specific ones:

```toml
[terminal.repl.profile.default.plugins.ldap]
provider = "uio-ldap-beta"
```

## Discovery Order

`osp` looks for plugins in this order:

1. `--plugin-dir <dir>`
2. `OSP_PLUGIN_PATH`
3. bundled plugin directory
4. `<platform-config-dir>/osp/plugins` (for example
   `~/.config/osp/plugins` on Linux)
5. `PATH` (`osp-*` executables) only when
   `extensions.plugins.discovery.path = true`

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

The persisted value is `plugins.<command>.provider = "<plugin-id>"` in the
active config scope.

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

Ambient `PATH` discovery is disabled by default. Turn it on explicitly if
you want `osp` to discover `osp-*` executables from your shell path:

```bash
osp config set extensions.plugins.discovery.path true
```

If a plugin times out:

1. run `osp plugins doctor`
2. retry with `-d`
3. increase `extensions.plugins.timeout_ms` if the backend is expected to be slow

## More Plugin Docs

- Authoring guide: [WRITING_PLUGINS.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/WRITING_PLUGINS.md)
- Packaging and manifests: [PLUGIN_PACKAGING.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/PLUGIN_PACKAGING.md)
- Protocol details: [PLUGIN_PROTOCOL.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/PLUGIN_PROTOCOL.md)
