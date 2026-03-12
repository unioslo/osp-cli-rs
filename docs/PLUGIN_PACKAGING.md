# Plugin Packaging

This document defines how to ship `osp` with optional bundled plugins.

## Distribution Modes

- Base: `osp`
  - Backbone only, no bundled plugins.
- Internal distro: `osp-uio`
  - Same `osp` binary plus bundled `osp-*` plugin executables.

## Discovery Order

Discovery order is deterministic, but command dispatch no longer auto-selects the
first provider when multiple active plugins expose the same command.

1. `--plugin-dir <dir>` (explicit CLI override)
2. `OSP_PLUGIN_PATH` (colon-separated directories)
3. Bundled plugins dir (package-owned)
4. `<platform-config-dir>/osp/plugins` (for example
   `~/.config/osp/plugins` on Linux)
5. `PATH` (`osp-*` executables) only when
   `extensions.plugins.discovery.path = true`

Conflict rule:
- If exactly one active plugin provides a command, dispatch uses it.
- If multiple active plugins provide a command, dispatch requires either:
  - a one-shot override via `osp <command> --plugin-provider <plugin-id> ...`
    (`--plugin-provider` is accepted anywhere before `--`)
  - or a persisted default via `osp plugins select-provider <command> <plugin-id>`
- `osp plugins commands` and REPL help/completion surface unresolved conflicts
  without inventing a merged grammar.
- Conflicts are surfaced by `osp plugins doctor`.

## Bundled Layout (Suggested)

```text
<install-root>/
  bin/osp
  lib/osp/plugins/
    osp-uio-ldap
    osp-uio-mreg
    manifest.toml
```

## Manifest (V1)

```toml
protocol_version = 1

[[plugin]]
id = "uio-ldap"
exe = "osp-uio-ldap"
version = "0.1.0"
enabled_by_default = true
checksum_sha256 = "..."
commands = ["ldap"]

[[plugin]]
id = "uio-mreg"
exe = "osp-uio-mreg"
version = "0.1.0"
enabled_by_default = true
checksum_sha256 = "..."
commands = ["mreg"]
```

## Manifest Validation Rules (Current)

When a bundled plugin directory is used, `osp` validates each plugin against
`manifest.toml`:

- `manifest.toml` must exist for bundled plugin directories that contain
  `osp-*` binaries.
- `protocol_version` must be `1`.
- `plugin.id`, `plugin.exe`, `plugin.version` must be non-empty.
- `plugin.commands` must not be empty.
- `plugin.id` and `plugin.exe` must be unique in the manifest.
- If `checksum_sha256` is set, executable SHA-256 must match before
  `osp` runs the plugin for `--describe`.
- Plugin `--describe` output must then match manifest `id`, `version`,
  and command list.

On mismatch, plugin is marked unhealthy and excluded from command dispatch.

## Scoped Command State

Plugin routing is regular config now, not a sidecar JSON file. State is scoped
by the same profile and terminal rules as the rest of the config.

Example:

```toml
[profile.default.plugins.ldap]
state = "enabled"
provider = "uio-ldap"

[terminal.repl.profile.default.plugins.ldap]
provider = "uio-ldap-beta"
```

Backbone behavior:
- `plugins.<command>.state = "enabled" | "disabled"` controls whether the
  command is available in the active scope.
- `plugins.<command>.provider = "<plugin-id>"` selects the preferred provider
  when multiple healthy plugins expose the same command.
- More specific scopes override less specific scopes.

## Operational Commands

- `osp plugins list`
- `osp plugins commands`
- `osp plugins enable <command>`
- `osp plugins disable <command>`
- `osp plugins clear-state <command>`
- `osp plugins select-provider <command> <plugin-id>`
- `osp plugins clear-provider <command>`
- `osp plugins doctor`
