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
4. `~/.config/osp/plugins`
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

## Local Enable/Disable State

Current scaffold stores local toggles in `~/.config/osp/plugins.json`.
This can migrate to TOML later if needed.

Example:

```json
{
  "enabled": ["uio-ldap"],
  "disabled": ["uio-mreg"],
  "preferred_providers": {
    "ldap": "uio-ldap"
  }
}
```

Backbone behavior:
- Enabled list takes priority when non-empty.
- Disabled list always excludes matched plugins.

## Operational Commands

- `osp plugins list`
- `osp plugins commands`
- `osp plugins enable <plugin-id>`
- `osp plugins disable <plugin-id>`
- `osp plugins select-provider <command> <plugin-id>`
- `osp plugins clear-provider <command>`
- `osp plugins doctor`
