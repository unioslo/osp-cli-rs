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
  - Plugin receives command arguments from backbone.
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
      "subcommands": ["user", "netgroup"]
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
