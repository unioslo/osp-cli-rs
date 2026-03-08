# Writing Plugins

This guide covers how to write an osp plugin, wire it into discovery,
and package it for distribution.

## How plugins work

A plugin is an executable that:

1. Responds to `--describe` with a JSON capability declaration
2. Receives commands as arguments and writes JSON responses to stdout
3. Is discovered by osp via filesystem search or manifest

The protocol is subprocess-based. Any language works as long as the
binary handles stdin/stdout JSON correctly.

## Minimal example

A plugin that provides an `echo` command:

```bash
#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--describe" ]]; then
    cat <<'EOF'
{
  "protocol_version": 1,
  "plugin_id": "my-echo",
  "plugin_version": "0.1.0",
  "commands": [
    {
      "name": "echo",
      "about": "Echo back the arguments",
      "args": [
        { "name": "text", "about": "Text to echo", "multi": true }
      ]
    }
  ]
}
EOF
    exit 0
fi

# Skip the command name (first arg is "echo"), echo the rest
shift
TEXT="${*:-hello}"

cat <<EOF
{
  "protocol_version": 1,
  "ok": true,
  "data": [{ "message": "$TEXT" }],
  "error": null,
  "messages": [],
  "meta": {
    "format_hint": null,
    "columns": ["message"],
    "column_align": []
  }
}
EOF
```

Save as `osp-my-echo`, make it executable, and place it in your PATH.
Then:

```bash
osp echo hello world
```

## Protocol reference

### Describe (capability declaration)

When invoked with `--describe`, a plugin must print a `DescribeV1` JSON
object to stdout and exit 0.

```json
{
  "protocol_version": 1,
  "plugin_id": "my-plugin",
  "plugin_version": "0.1.0",
  "min_osp_version": null,
  "commands": [
    {
      "name": "mycommand",
      "about": "Short description",
      "args": [
        {
          "name": "target",
          "about": "What to query",
          "multi": false,
          "suggestions": [
            { "value": "users", "meta": "Query users" },
            { "value": "groups", "meta": "Query groups" }
          ]
        }
      ],
      "flags": {
        "--verbose": {
          "about": "Show extra detail",
          "flag_only": true
        },
        "--limit": {
          "about": "Max results",
          "flag_only": false
        }
      },
      "subcommands": []
    }
  ]
}
```

#### DescribeV1 fields

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `protocol_version` | integer | yes | Must be `1` |
| `plugin_id` | string | yes | Unique identifier, non-empty |
| `plugin_version` | string | yes | Semantic version |
| `min_osp_version` | string | no | Minimum osp version required |
| `commands` | array | yes | At least one command |

#### Command fields

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `name` | string | required | Top-level command name |
| `about` | string | `""` | Short help text |
| `args` | array | `[]` | Positional arguments |
| `flags` | object | `{}` | Named flags (keys start with `--`) |
| `subcommands` | array | `[]` | Nested subcommands |

#### Argument fields

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `name` | string | `null` | Display name for help |
| `about` | string | `null` | Help text |
| `multi` | bool | `false` | Accepts multiple values |
| `value_type` | string | `null` | `"Path"` for path completion |
| `suggestions` | array | `[]` | Tab completion values |

#### Flag fields

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `about` | string | `null` | Help text |
| `flag_only` | bool | `false` | `true` = boolean flag, `false` = takes a value |
| `multi` | bool | `false` | Can be repeated |
| `value_type` | string | `null` | `"Path"` for path completion |
| `suggestions` | array | `[]` | Tab completion values |

### Execute (command invocation)

For normal command execution, the plugin receives:

- **argv**: `<plugin-exe> <command> [args...]`
  For example: `osp-my-plugin mycommand alice --verbose` arrives as
  `argv = ["osp-my-plugin", "mycommand", "alice", "--verbose"]`
- **stdin**: not used (reserved for future protocol extensions)
- **stdout**: must write a `ResponseV1` JSON object
- **stderr**: free-form diagnostic output (shown to user on failure)

#### ResponseV1

```json
{
  "protocol_version": 1,
  "ok": true,
  "data": [
    { "uid": "alice", "cn": "Alice Smith", "mail": "alice@example.com" },
    { "uid": "bob", "cn": "Bob Jones", "mail": "bob@example.com" }
  ],
  "error": null,
  "messages": [
    { "level": "info", "text": "Found 2 results" }
  ],
  "meta": {
    "format_hint": null,
    "columns": ["uid", "cn", "mail"],
    "column_align": []
  }
}
```

#### ResponseV1 fields

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `protocol_version` | integer | yes | Must be `1` |
| `ok` | bool | yes | `true` = success, `false` = error |
| `data` | any | yes | Array of row objects for table display, or any JSON value |
| `error` | object | if `ok=false` | Error details (see below) |
| `messages` | array | no | Diagnostic messages shown to user |
| `meta` | object | yes | Output metadata |

#### Validation rules

- If `ok` is `true`, `error` must be `null`
- If `ok` is `false`, `error` must be present
- `protocol_version` must be `1`

#### Error object

```json
{
  "code": "NOT_FOUND",
  "message": "User alice not found",
  "details": {}
}
```

| Field | Type | Notes |
|-------|------|-------|
| `code` | string | Machine-readable error code |
| `message` | string | Human-readable error message |
| `details` | any | Optional structured error data |

#### Messages

Messages are shown to the user alongside the output:

| Level | When to use |
|-------|-------------|
| `error` | Something failed |
| `warning` | Something unexpected but not fatal |
| `success` | Positive confirmation |
| `info` | Neutral information |
| `trace` | Debug-level detail (shown with `-v`) |

#### Meta fields

| Field | Type | Notes |
|-------|------|-------|
| `format_hint` | string | Suggest output format (`"table"`, `"mreg"`, etc.) |
| `columns` | array | Column order for table display |
| `column_align` | array | Per-column alignment: `"default"`, `"left"`, `"center"`, `"right"` |

### Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 2 | Usage/argument error |
| 10 | Resource not found |
| 20 | Auth/config error |
| 30 | Upstream API failure |
| 40 | Internal error |

Non-zero exit with no valid JSON on stdout is treated as a plugin
crash. The stderr output is shown to the user.

## Environment variables

osp injects environment variables into the plugin process:

### Runtime hints

| Variable | Values | Notes |
|----------|--------|-------|
| `OSP_UI_VERBOSITY` | `error`, `warning`, `success`, `info`, `trace` | User's verbosity level |
| `OSP_DEBUG_LEVEL` | `0`-`3` | Debug verbosity from `-d` flags |
| `OSP_FORMAT` | `auto`, `json`, `table`, `md`, `mreg`, `value` | Requested output format |
| `OSP_COLOR` | `auto`, `always`, `never` | Color preference |
| `OSP_UNICODE` | `auto`, `always`, `never` | Unicode preference |
| `OSP_TERMINAL_KIND` | `cli`, `repl`, `unknown` | Whether running in CLI or REPL |
| `OSP_PROFILE` | profile name | Active config profile (if set) |
| `OSP_TERMINAL` | terminal name | Raw `TERM` value (if set) |

### Plugin-specific

| Variable | Notes |
|----------|-------|
| `OSP_COMMAND` | The top-level command being executed |

### Config-driven plugin env

Users can pass config values to plugins via the config file:

```toml
[default]
extensions.plugins.env.api_url = "https://api.example.com"

[default]
extensions.plugins.my-plugin.env.token = "secret"
```

These become environment variables:

- `extensions.plugins.env.api_url` becomes `OSP_PLUGIN_CFG_API_URL`
- `extensions.plugins.my-plugin.env.token` becomes
  `OSP_PLUGIN_CFG_TOKEN` (only for `my-plugin`)

Plugin-specific values override shared values for the same key.

## Discovery

osp searches for plugins in this order:

1. `--plugin-dir <dir>` (CLI flag, repeatable)
2. `OSP_PLUGIN_PATH` (colon-separated directories)
3. Bundled plugin directories:
   - `OSP_BUNDLED_PLUGIN_DIR` env var
   - `<osp-binary-dir>/plugins`
   - `<osp-binary-dir>/../lib/osp/plugins`
4. `~/.config/osp/plugins/` (user plugin directory)
5. `PATH` (searches for `osp-*` executables) only when
   `extensions.plugins.discovery.path = true`

The simplest way to make a plugin available: name it `osp-<something>`,
make it executable, and put it in an explicit plugin directory, in
`OSP_PLUGIN_PATH`, or in `PATH` after enabling path discovery.

### Discovery caching

Plugin `--describe` results are cached in
`~/.cache/osp/describe-v1.json`, keyed by executable path, file size,
and modification time. The cache is invalidated automatically when the
binary changes.

Force a cache refresh:

```bash
osp plugins refresh
```

## Packaging with a manifest

For bundled distribution, plugins use a `manifest.toml` file placed
alongside the executables:

```toml
protocol_version = 1

[[plugin]]
id = "my-plugin"
exe = "osp-my-plugin"
version = "0.1.0"
enabled_by_default = true
checksum_sha256 = "abc123..."
commands = ["mycommand"]
```

### Manifest fields

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `protocol_version` | integer | yes | Must be `1` |
| `plugin[].id` | string | yes | Unique plugin identifier |
| `plugin[].exe` | string | yes | Executable filename (no path) |
| `plugin[].version` | string | yes | Semantic version |
| `plugin[].enabled_by_default` | bool | no | Default: `false` |
| `plugin[].checksum_sha256` | string | no | SHA-256 of executable |
| `plugin[].commands` | array | yes | List of top-level commands |

### Validation

When a manifest is present:
- If `checksum_sha256` is set, the executable's SHA-256 must match
  before `osp` runs `--describe`
- The executable's `--describe` output must match the manifest's `id`,
  `version`, and command list
- IDs and executable names must be unique within a manifest

### Bundled layout

```
lib/osp/plugins/
  manifest.toml
  osp-my-plugin
  osp-other-plugin
```

## Plugin management

Users manage plugins with built-in commands:

```bash
osp plugins list              # Show discovered plugins
osp plugins commands          # Show plugin-provided commands
osp plugins enable my-plugin  # Enable a plugin
osp plugins disable my-plugin # Disable a plugin
osp plugins doctor            # Diagnose plugin issues
osp plugins refresh           # Clear discovery cache
```

### Provider conflicts

When multiple plugins provide the same command, osp does not guess. The
command becomes ambiguous until the user either picks a provider for the
current invocation or stores a preferred provider:

```bash
osp ldap user alice --plugin-provider uio-ldap
osp plugins select-provider ldap uio-ldap
osp plugins clear-provider ldap
```

Plugin state is stored in `~/.config/osp/plugins.json`.

## Writing a plugin in Rust

For Rust plugins, you can use clap for argument parsing and serde for
JSON serialization. The `osp-core` crate exports the protocol types:

```rust
use osp_core::plugin::{DescribeV1, ResponseV1, ResponseMetaV1};
```

A Rust plugin can use `DescribeV1::from_clap_command()` to generate
the describe output from a clap `Command` definition automatically,
keeping the CLI and protocol in sync.

## Writing a plugin in Python

```python
#!/usr/bin/env python3
import json
import sys
import os

def describe():
    return {
        "protocol_version": 1,
        "plugin_id": "py-example",
        "plugin_version": "0.1.0",
        "commands": [
            {
                "name": "greet",
                "about": "Greet a user",
                "args": [{"name": "name", "about": "Name to greet"}],
            }
        ],
    }

def execute(args):
    name = args[0] if args else "world"
    return {
        "protocol_version": 1,
        "ok": True,
        "data": [{"greeting": f"Hello, {name}!"}],
        "error": None,
        "messages": [],
        "meta": {"format_hint": None, "columns": ["greeting"], "column_align": []},
    }

if __name__ == "__main__":
    if "--describe" in sys.argv:
        json.dump(describe(), sys.stdout)
    else:
        # argv[0] = script, argv[1] = command name, argv[2:] = args
        result = execute(sys.argv[2:])
        json.dump(result, sys.stdout)
```

Save as `osp-py-example`, `chmod +x`, add to PATH.

## Debugging

Run `osp plugins doctor` to diagnose discovery and protocol issues.

Test your plugin manually:

```bash
# Check describe output
./osp-my-plugin --describe | python3 -m json.tool

# Check command output
./osp-my-plugin mycommand alice | python3 -m json.tool
```

Use `osp --debug` to see plugin invocation details (when
instrumentation is enabled).

## Timeout

Plugins have a default process timeout of 10 seconds. If your plugin
needs longer (large queries, slow APIs), consider streaming partial
results or increasing the timeout via configuration.
