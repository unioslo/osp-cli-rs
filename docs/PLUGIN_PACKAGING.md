# Plugin Packaging

This document defines how to ship `osp` with optional bundled plugins.

## Distribution Modes

- Base: `osp`
  - Backbone only, no bundled plugins.
- Internal distro: `osp-uio`
  - Same `osp` binary plus bundled `osp-*` plugin executables.

## Discovery Order

Discovery is deterministic and stops on first provider for a command.

1. `--plugin-dir <dir>` (explicit CLI override)
2. `OSP_PLUGIN_PATH` (colon-separated directories)
3. Bundled plugins dir (package-owned)
4. `~/.config/osp/plugins`
5. `PATH` (`osp-*` executables)

Conflict rule:
- First match wins.
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
- Plugin `--describe` output must match manifest `id`, `version`, and command list.
- If `checksum_sha256` is set, executable SHA-256 must match.

On mismatch, plugin is marked unhealthy and excluded from command dispatch.

## Local Enable/Disable State

Current scaffold stores local toggles in `~/.config/osp/plugins.json`.
This can migrate to TOML later if needed.

Example:

```json
{
  "enabled": ["uio-ldap"],
  "disabled": ["uio-mreg"]
}
```

Backbone behavior:
- Enabled list takes priority when non-empty.
- Disabled list always excludes matched plugins.

## Operational Commands

- `osp plugins list`
- `osp plugins enable <plugin-id>`
- `osp plugins disable <plugin-id>`
- `osp plugins doctor`
