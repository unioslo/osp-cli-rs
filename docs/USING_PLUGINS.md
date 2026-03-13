# Using Plugins

Most domain commands in `osp` come from plugins.

If you are only using built-in commands, you can skip this file.

Examples that use `inventory ...` below are illustrative provider-backed
command shapes. Replace them with a real command from `osp plugins commands`.

## Inspect Plugins

Useful commands:

- `osp plugins list`
- `osp plugins commands`
- `osp plugins doctor`
- `osp plugins refresh`

Use `plugins doctor` first when a command is missing or behaving unexpectedly.

## Enable and Disable Commands

```bash
osp plugins enable inventory
osp plugins disable inventory
osp plugins clear-state inventory
```

These commands persist command routing in the scoped config file. `enable` and
`disable` set `state = "enabled" | "disabled"`, and `clear-state` removes the
explicit override so the command falls back to plugin defaults.

Example:

```toml
[profile.default.plugins.inventory]
state = "enabled"
provider = "inventory-a"
```

More specific terminal and profile scopes override less specific ones:

```toml
[terminal.repl.profile.default.plugins.inventory]
provider = "inventory-beta"
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

PATH discovery is intentionally passive. During ordinary discovery, `osp` does
not execute PATH plugins just to ask for `--describe`. That means:

- `plugins list` can show a PATH plugin with no discovered commands yet
- `plugins commands` can stay empty until the first real dispatch or a cached
  describe payload exists

## Provider Conflicts

If exactly one active plugin provides a command, `osp` uses it automatically.

If multiple active plugins provide the same command, `osp` does not guess.
Choose a provider for one invocation:

```bash
osp inventory host web-01 --plugin-provider inventory-a
```

Or store a preferred provider:

```bash
osp plugins select-provider inventory inventory-a
osp plugins clear-provider inventory
```

The persisted value is `plugins.<command>.provider = "<plugin-id>"` in the
active config scope.

`osp plugins commands` and REPL help/completion show unresolved conflicts
instead of inventing a merged command grammar.

## REPL Usage

The same provider-selection rules apply in the REPL:

```text
inventory host web-01 --plugin-provider inventory-a
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

- Authoring guide: [WRITING_PLUGINS.md](WRITING_PLUGINS.md)
- Packaging and manifests: [PLUGIN_PACKAGING.md](PLUGIN_PACKAGING.md)
- Protocol details: [PLUGIN_PROTOCOL.md](PLUGIN_PROTOCOL.md)
