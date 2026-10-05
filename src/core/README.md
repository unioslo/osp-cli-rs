# core

Small shared types that must mean the same thing everywhere: a row, a command
result, a command description, the plugin wire DTOs, shell quoting and fuzzy
matching. Everything else builds on these.

Imports nothing else from the crate, and must stay that way. A type belongs
here only if several modules need it and its meaning will not change with any
one feature. `core` is not a place for helpers without a home.

## Layout

| File | Owns |
|---|---|
| `row.rs` | `Row`, the record type commands, the DSL and the UI pass around |
| `output_model.rs` | `OutputResult`: rows, documents, groups and render hints |
| `output.rs` | Output format and mode enums shared by flags, config and UI |
| `command_def.rs` | `CommandDef`, `FlagDef`, `ArgDef`: commands as data for help, completion and catalogs |
| `command_policy.rs` | Whether a command is visible or allowed in the current context |
| `plugin.rs` | Plugin protocol DTOs (`DescribeV1`, `ResponseV1`) and validation |
| `runtime.rs` | Runtime hints passed to native and plugin commands |
| `shell_words.rs`, `fuzzy.rs` | Quoting for display; the fuzzy matcher used by completion |

## Common changes

- **Protocol change:** `plugin.rs` together with `docs/PLUGIN_PROTOCOL.md`.
  Additive fields with defaults keep old plugins working; anything else needs
  a protocol version bump.
- **New command metadata (e.g. a flag property):** add it to `command_def.rs`,
  then carry it through `completion/tree.rs` and `guide` as needed. Clap-built
  commands get it from the clap conversion in the same file.
