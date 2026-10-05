# plugin

Lets external executables provide `osp` commands. A plugin answers `describe`
with JSON; from then on the rest of the host sees ordinary command metadata
and only this module ever spawns the process.

Imports `core` (wire DTOs in `core/plugin.rs`), `config`, `completion` and
`native`, plus `app::ConfigState` in `config.rs`. Rendering, the DSL and the
REPL stay outside.

## Layout

| File | Owns |
|---|---|
| `manager.rs` | `PluginManager`, the facade the host uses |
| `discovery.rs` | Scanning roots, the describe cache, when describing is allowed |
| `active.rs`, `catalog.rs` | The working set and the command catalog/doctor views built from it |
| `selection.rs` | Picking a provider when two plugins export the same command |
| `state.rs` | Persisted per-command preferences (enabled, selected provider) |
| `dispatch.rs` | Running a plugin command and validating its `ResponseV1` |
| `config.rs` | Projecting config into a plugin's environment |
| `conversion.rs` | `DescribeCommandV1` into a completion `CommandSpec` |

## Boundaries

- The wire format lives in `core/plugin.rs` and `docs/PLUGIN_PROTOCOL.md`.
  Change both together and bump the protocol version for breaking changes.
- In-process commands that should look like plugins use `native`, not a
  subprocess shim.
- Plugins can declare visibility and auth hints; the host policy decides, and
  the service the plugin calls stays authoritative.

## Common operations

```sh
osp plugins list                     # what was discovered, and from where
osp plugins commands                 # the resulting catalog
osp plugins doctor                   # conflicts and unhealthy plugins
osp plugins select-provider <command> <plugin-id>
osp plugins refresh                  # ignore the describe cache once
```

See `docs/WRITING_PLUGINS.md` for the plugin side.
