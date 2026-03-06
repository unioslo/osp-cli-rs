# Config and Profile Resolution

This is a deliberately simpler and more deterministic design than the current
osprov-cli config system. It keeps the good parts (multi-source, profile +
terminal scoping, placeholders, secrets isolation) and removes the parts that
caused complexity (context stabilization loops, callable values in config).

## What We Keep From osprov-cli

The current system has strong ideas worth preserving:

- Multiple sources with explicit precedence.
- Two scoping axes with predictable resolution order.
- Placeholder interpolation for paths and derived strings.
- Separate secrets backend with strict permissions.
- Diagnostics that show where values came from.

## What We Do Differently

- Profile is selected before config resolution. No resolver loops to “stabilize”
  a profile. This removes oscillation risk entirely.
- Config values are static. No callables stored in config files or defaults.
  Derived values are computed in code after resolution.
- Loader precedence is “last wins” and is listed in the docs. No hidden priority
  math or “first wins” merging.
- No default secrets in code. Tokens and passwords must come from secrets,
  env vars, or CLI flags.
- Strict schema by default. Unknown keys are rejected unless explicitly allowed
  under an `extensions.*` namespace.

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

- Launch-time flags that map cleanly to config keys are seeded into the
  in-memory session layer.
  - examples: `--json`, `--mode`, `--color`, `--ascii`, `--theme`, `-u`,
    `-v/-q`, `-d`
- REPL `config set --session ...` writes into that same layer.
- This means some launch flags may appear as `source=session` in
  `config get --sources`, which is intentional.

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
- Values are adapted to schema type after merge/interpolation:
  strings, booleans, integers, floats.
- Enum-like string keys (for example `ui.format`) are validated against
  allowed values.
- Required keys are enforced (`profile.default`, `profile.active`).

## Resolution Pipeline

1. Select profile from CLI or use `profile.default`.
2. Load all sources and apply scope resolution inside each loader.
3. Merge sources using the precedence list above.
4. Interpolate placeholders in string values.
5. Adapt types using the schema.
6. Compute derived values in code, not in config.
7. Validate required keys and freeze the result.

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
ui.prompt.secrets = true

[terminal.repl.profile.tsd]
ui.format = "table"
```

Notes:

- The profile list is derived from the `[profile.*]` tables.
- Unscoped keys live in `[default]`.
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

## Validation and Diagnostics

The resolver should provide:

- `config show` for resolved values
- `config show --sources` to include source + scope
- `config show --raw` for pre-interpolation values
- `config get <key>` and `config get <key> --sources`
- `config explain <key>` for winner + precedence chain + interpolation trace
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

- `theme.name`
- `theme.path`
- `user.name`
- `domain`
- `repl.prompt`
- `repl.simple_prompt`
- `repl.shell_indicator`
- `repl.intro`
- `color.prompt.text`
- `color.prompt.command`
- `ui.messages.format`
