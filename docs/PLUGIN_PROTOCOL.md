# Plugin Protocol V1

This document defines the executable plugin protocol for `osp`.
Plugins are separate binaries discovered at runtime (Option B).

## Goals

- Keep plugin integration stable across independent releases.
- Let backbone CLI/repl render plugin output consistently.
- Avoid in-process ABI coupling.

## Process Model

- Backbone binary: `osp`
- Plugin binaries: `osp-<id>` (for example `osp-uio-ldap`)
- Transport: subprocess invocation + JSON over stdout

## Required Plugin Commands

- `--describe`
  - Prints `DescribeV1` JSON to stdout.
  - Exit code 0 on success.
- Normal execution
  - Backbone resolves the plugin from the selected top-level command and then
    invokes the plugin as `<plugin-exe> <selected-command> <remaining-argv...>`.
  - Example: `osp ldap user oistes` invokes the plugin with argv
    `["ldap", "user", "oistes"]`.
  - Backbone also sets `OSP_COMMAND=<selected-top-level-command>`.
  - `OSP_COMMAND` and `argv[1]` carry the same selected command on purpose.
    Plugins may use either, but they should agree.
  - Prints `ResponseV1` JSON to stdout.
  - Uses non-zero exit for process-level failure.

## Help Delegation

- `osp <plugin-command> --help` and `osp <plugin-command> help` are passed
  through directly to the plugin process.
- For delegated help, backbone does not require `ResponseV1` JSON.

## Describe Caching (Backbone Behavior)

- Backbone caches successful `--describe` payloads.
- Cache key is `(executable path, file size, file mtime)`.
- Default cache file: `~/.cache/osp/describe-v1.json`.
- If `XDG_CACHE_HOME` is set, cache path is
  `$XDG_CACHE_HOME/osp/describe-v1.json`.

## DescribeV1

```json
{
  "protocol_version": 1,
  "plugin_id": "uio-ldap",
  "plugin_version": "0.1.0",
  "min_osp_version": "0.1.0",
  "commands": [
    {
      "name": "ldap",
      "about": "LDAP lookups",
      "subcommands": [
        { "name": "user", "about": "Lookup LDAP users", "subcommands": [] },
        { "name": "netgroup", "about": "Lookup LDAP netgroups", "subcommands": [] }
      ]
    }
  ]
}
```

Rules:
- `protocol_version` must be exactly `1`.
- `plugin_id` must be unique within discovery scope.
- `commands[].name` is top-level command claimed by the plugin.

## ResponseV1

```json
{
  "protocol_version": 1,
  "ok": true,
  "data": {},
  "error": null,
  "messages": [
    { "level": "info", "text": "Using profile: uio" }
  ],
  "meta": {
    "format_hint": "table",
    "columns": ["uid", "cn"]
  }
}
```

Rules:
- `protocol_version` must be exactly `1`.
- `ok=true` implies `error=null`.
- `ok=false` implies `error` is present.
- `data` is always present (empty object/array is allowed).
- `messages` is optional and defaults to an empty list.

Message levels:
- `error`
- `warning`
- `success`
- `info`
- `trace`

Backbone behavior:
- plugin `messages` are rendered by `osp-ui` on stderr using the same
  grouping/theme/verbosity rules as built-in commands.
- plugin data remains on stdout.

## Error Shape

```json
{
  "code": "AUTH_FAILED",
  "message": "LDAP bind failed",
  "details": {}
}
```

## Exit Code Guidance

- `0`: success
- `2`: usage/argument error
- `10`: plugin not found (backbone-level)
- `20`: auth/config error
- `30`: upstream API failure
- `40`: internal error

## Compatibility Policy

- Backbone rejects unsupported `protocol_version`.
- Backbone may reject plugins below required `min_osp_version`.
- New fields must be additive and optional.

## Runtime Hints Environment

Backbone injects runtime hints into each plugin subprocess. Plugins can use
`osp_core::runtime::RuntimeHints::from_env()` to parse them.

Required hints:
- `OSP_UI_VERBOSITY=error|warning|success|info|trace`
- `OSP_DEBUG_LEVEL=0|1|2|3`
- `OSP_FORMAT=auto|json|table|md|mreg|value`
- `OSP_COLOR=auto|always|never`
- `OSP_UNICODE=auto|always|never`
- `OSP_TERMINAL_KIND=cli|repl|unknown`

Optional hints:
- `OSP_PROFILE=<active-profile>`
- `OSP_TERMINAL=<raw-TERM-value>`

Additional process env:
- `OSP_COMMAND=<selected-top-level-command>`

## Config-Driven Plugin Env

Backbone can also project selected config values into plugin subprocess env.
This is intended for plugin-specific settings owned by the app config, not for
the runtime hints above.

Config namespaces:
- shared across all plugins:
  - `extensions.plugins.env.<name> = <value>`
- scoped to one plugin id:
  - `extensions.plugins.<plugin-id>.env.<name> = <value>`

Env mapping rules:
- `<name>` is normalized to uppercase with non-alphanumeric characters replaced
  by `_`.
- Backbone prefixes the name with `OSP_PLUGIN_CFG_`.
- Example:
  - `extensions.plugins.env.api.url = "https://example"`
  - `extensions.plugins.ldap.env.bind.password = "sekrit"`
  - become `OSP_PLUGIN_CFG_API_URL` and `OSP_PLUGIN_CFG_BIND_PASSWORD`
- plugin-specific values override shared values when they map to the same env
  name.
- app-owned plugin config is injected after runtime hints, so later
  `OSP_PLUGIN_CFG_*` values intentionally win if a plugin reuses the same env
  name across shared and plugin-specific config.
- scalar config values are stringified directly.
- list values are encoded as JSON arrays.

Examples:
- `extensions.plugins.env.api.url = "https://common.example"`
- `extensions.plugins.ldap.env.bind.password = "sekrit"`
