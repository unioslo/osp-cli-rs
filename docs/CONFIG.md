# Config and Profile Resolution

`osp` resolves config from multiple sources with explicit precedence, profile
scoping, terminal scoping, and separate secret storage.

## Overview

Key properties:

- multiple sources with explicit precedence
- profile and terminal scoping
- placeholder interpolation for strings
- separate secret storage
- diagnostics that show where values came from
- strict schema validation with `extensions.*` as the open namespace

## Data Model

Each value is tracked with its origin and scope:

- `ConfigKey` = `key`, `profile`, `terminal`
- `ConfigValue` = raw value + source + scope
- `ResolvedValue` = final value + raw value + source + scope + origin hint

Scopes are always optional. Unscoped values act as the baseline.

## Scoping Rules (Profile + Terminal)

Within a single loader, the resolver picks the most specific match using this
order:

1. (key, profile, terminal)
2. (key, profile, none)
3. (key, none, terminal)
4. (key, none, none)

This rule is applied uniformly to all loaders.

## Loader Precedence

Lowest to highest priority, later sources override earlier ones:

1. Built-in defaults
2. Config file
3. Secrets store
4. Environment variables
5. Session overrides (in-memory)

This order is the public contract. Changing it is a breaking change.

Notes:

- REPL `config set --session ...` writes into the in-memory session layer.
- Launch-time bootstrap flags like `--theme` and `-u` may still affect startup
  state or session defaults.
- Invocation flags like `--json`, `--format`, `--mode`, `--color`, `--ascii`,
  `-v/-q`, `-d`, and `--plugin-provider` do not write into config state.
- `config get --sources` therefore reflects stored defaults, not one-shot
  invocation overrides.

For UI keys, there is one more rule inside the resolved config:

- explicit per-key values beat `ui.presentation`
- `ui.presentation` only seeds keys still at builtin default
- `config explain` shows that seeded effect when it matters

## Loader Abstraction

`osp-config` exposes a generic loader interface so each source can be wired
without custom code in `osp-cli`:

- `ConfigLoader` trait (`load() -> ConfigLayer`)
- `StaticLayerLoader` for in-memory defaults/session layers
- `TomlFileLoader` for file-backed layers (optional or required)
- `EnvVarLoader` for `OSP__...` environment overrides
- `SecretsTomlLoader` for file-backed secrets with strict permissions checks
- `EnvSecretsLoader` for `OSP_SECRET__...` secret overrides
- `ChainedLoader` for combining multiple backends within one precedence stage
- `LoaderPipeline` for assembling all loaders in precedence order

## Typed Schema and Validation

`osp-config` validates resolved keys against a typed schema (`ConfigSchema`):

- Unknown keys are rejected by default.
- `extensions.*` is the only open namespace for unknown keys.
- Plugin-owned config can live under `extensions.plugins.*`; the CLI projects
  `extensions.plugins.env.*` and `extensions.plugins.<plugin-id>.env.*` into
  plugin subprocess env. Inspect the effective projection with
  `osp plugins config <plugin-id>`. See `docs/PLUGIN_PROTOCOL.md`.
- `extensions.plugins.timeout_ms` bounds plugin discovery (`--describe`) and
  command execution. Default: `10000`.
- `extensions.plugins.discovery.path` enables discovery of `osp-*` executables
  from ambient `PATH`. Default: `false`.
- Values are adapted to schema type after merge/interpolation:
  strings, booleans, integers, floats.
- Enum-like string keys (for example `ui.format`) are validated against
  allowed values.
- Required runtime keys are enforced (`profile.active`).
- Bootstrap-only keys are validated during bootstrap, not treated as ordinary
  runtime-resolved values.

## Resolution Pipeline

1. Run path bootstrap from pre-file inputs:
   CLI/env/session/bootstrap context/platform defaults.
2. Load all configured layers.
3. Run profile bootstrap:
   CLI `--profile` override or `profile.default` from bootstrap-safe scopes.
4. Merge runtime values using the precedence list above.
5. Interpolate placeholders in string values.
6. Adapt types using the schema.
7. Compute derived values in code, not in config.
8. Validate required runtime keys and freeze the result.

Bootstrap is expressive by layer, but restricted by scope:

- path bootstrap does not read config-file contents
- profile bootstrap reads across loaded layers
- profile bootstrap ignores profile-dependent scopes
- runtime resolution never changes the active profile

## Placeholders

Placeholders are string-only `${key}` references and are resolved after all
sources are merged. Rules:

- Unresolved placeholders are errors.
- Cycles are errors.
- Placeholders can reference other placeholders.
- Placeholders may reference secrets but must be redacted in diagnostics.

## TOML Structure

This is the canonical file layout:

```toml
[default]
profile.default = "uio"
ui.format = "table"

[profile.uio]
osp.url = "https://osp-orchestrator.uio.no"

[profile.tsd]
ui.format = "json"

[terminal.repl]
profile.default = "tsd"
ui.prompt.secrets = true

[terminal.repl.profile.tsd]
ui.format = "table"
```

Notes:

- The profile list is derived from the `[profile.*]` tables.
- Unscoped keys live in `[default]`.
- Bootstrap keys such as `profile.default` may be set in `[default]` and
  `terminal.<term>`.
- Bootstrap keys such as `profile.default` must not be set in `[profile.*]` or
  `terminal.<term>.profile.<name>`.
- Fully scoped values use `terminal.<term>.profile.<name>`.

## Environment Variable Mapping

Keep the mapping explicit and predictable:

- `OSP__UI__FORMAT` -> `ui.format`
- `OSP__PROFILE__TSD__UI__FORMAT` -> `ui.format` scoped to profile `tsd`
- `OSP__TERM__REPL__UI__FORMAT` -> `ui.format` scoped to terminal `repl`
- `OSP__TERM__REPL__PROFILE__TSD__UI__FORMAT` -> fully scoped

All env values are strings and are adapted by the schema.
Profile and terminal identifiers are case-insensitive and normalized to
lowercase at parse time.

Secret env mapping uses the same scope grammar with `OSP_SECRET__`:

- `OSP_SECRET__LDAP__BIND_PASSWORD` -> `ldap.bind_password`
- `OSP_SECRET__PROFILE__TSD__LDAP__BIND_PASSWORD` -> profile-scoped secret
- `OSP_SECRET__TERM__REPL__PROFILE__TSD__LDAP__BIND_PASSWORD` ->
  terminal+profile scoped secret

## Secrets

Secrets are stored in a separate backend:

- Default backend: TOML file with `0600` permissions (`SecretsTomlLoader`).
- Optional override backend: `OSP_SECRET__...` environment variables
  (`EnvSecretsLoader`).
- Secrets are never stored in the main config file.
- Secrets are wrapped in a redacted type for diagnostics.

No secret defaults are allowed in code. Missing secrets must surface as
clear config errors.

## Derived Values

Derived values are computed after resolution and are not configurable:

- `app.config_dir`, `app.data_dir`, `app.log_dir`
- `profile.active`
- resolved prompt strings

Derived values must never depend on values that can change at runtime without
rebuilding the config.

`profile.default` is not a runtime-derived value. It is a bootstrap input and
is not carried as an ordinary resolved runtime key.

## Validation and Diagnostics

The resolver should provide:

- `config show` for resolved runtime values
- `config show --sources` to include source + scope
- `config show --raw` for pre-interpolation values
- `config get <key>` and `config get <key> --sources`
- `config explain <key>` for winner + precedence chain + interpolation trace
- bootstrap-aware explain for bootstrap keys such as `profile.default`
- explain output should label whether a key was resolved in bootstrap or runtime
- JSON explain output includes `"phase": "bootstrap"` or `"phase": "runtime"`
- bootstrap-only keys stay out of ordinary runtime `config show`
- `config diagnostics` summary

Diagnostics must redact secrets and avoid logging token contents.

## Testing Checklist

Minimum contract tests:

1. Profile scoping beats unscoped values.
2. Terminal scoping beats unscoped values.
3. Session overrides beat environment and config.
4. Placeholder cycles raise errors.
5. Unknown keys are rejected unless under `extensions.*`.

## UI/REPL Keys (Current)

These keys now drive REPL chrome and message rendering:

- `ui.presentation`
  - `expressive | compact | austere`
- `ui.mode`
- `ui.color.mode`
- `ui.unicode.mode`
- `ui.chrome.frame`
  - `none | top | bottom | top-bottom | square | round`
- `ui.table.border`
  - `none | square | round`
- `ui.table.overflow`
- `theme.name`
- `theme.path`
- `user.name`
- `domain`
- `repl.prompt`
- `repl.simple_prompt`
- `repl.shell_indicator`
- `repl.intro`
  - `none | minimal | compact | full`
- `ui.help.layout`
  - `full | compact | minimal`
- `color.prompt.text`
- `color.prompt.command`
- `ui.messages.layout`
  - `grouped | minimal`

## UI Examples

Compact REPL defaults:

```toml
[terminal.repl]
ui.presentation = "compact"
```

Quiet, plain operator profile:

```toml
[profile.ops]
ui.presentation = "austere"
ui.mode = "plain"
ui.color.mode = "never"
ui.chrome.frame = "none"
ui.messages.layout = "minimal"
```

Compatibility note:

- if you used `gammel-og-bitter` before, use `ui.presentation = "austere"`
- the old name remains a CLI alias, not the canonical config vocabulary

## REPL Config Writes

Store choice depends on where you run the command:

- in one-shot CLI, `config set` defaults to the persistent config store
- in the REPL, `config set` defaults to the session store
- use `--save`, `--config`, or `--secrets` for persistence
- use `--session` to force in-memory session behavior
