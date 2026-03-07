# Resolver Makeover Plan

This plan is for a ground-up rebuild of the `osp-config` resolver surface, not
just a narrow patch to `profile.default`.

The target is a Rust config system that is clearly better than the Python
`ResolverV2` in:

- semantics
- code clarity
- maintainability
- explain/debug behavior
- operational safety

The Rust version does not need to be more dynamic than the Python one. It needs
to be more disciplined.

## Non-Negotiable Direction

- [x] Keep the Rust model staged and deterministic.
- [x] Support dynamic config-file and profile selection only through explicit
      bootstrap inputs.
- [x] Keep runtime config resolution data-oriented: select winners, interpolate,
      validate, derive.
- [x] Reject Python-style fixed-point resolution, config callables, loader
      dependency graphs, and context stabilization loops.
- [x] Accept breaking changes where they make the semantics cleaner.

## Hard Invariants

These should be treated as contract rules, not implementation details.

- [x] bootstrap-only keys never appear as ordinary runtime resolved keys
- [x] runtime snapshots contain only runtime-visible keys
- [x] runtime resolution never changes the active profile
- [x] bootstrap keys are never resolved through profile-dependent scopes
- [x] interpolation operates only on already-selected raw winners
- [x] explain output is derived from the same phase-specific selection rules as
      normal resolution

The biggest one is the key-category boundary:

- [x] `profile.default` is bootstrap-only
- [x] `profile.active` is runtime-visible and derived

If a bootstrap value needs runtime visibility, that should happen through an
intentional derived mirror, not by leaking bootstrap keys into
`ResolvedConfig.values`.

## What Is Good In The Current Rust File

These parts are worth preserving:

- [ ] one resolution frame shared by resolve and explain
- [ ] centralized scope precedence in `ScopeSelector`
- [ ] two-step raw winner selection followed by interpolation/schema
- [ ] derived `profile.active`
- [ ] layered explain output with candidate provenance
- [ ] secret propagation through interpolation

The file is already conceptually stronger than the Python resolver. The
makeover should sharpen that advantage, not throw it away.

## What Needs To Change

### Semantic problems

- [ ] `profile.default` currently behaves like both a bootstrap key and a normal
      resolved key.
- [ ] bootstrap path selection and bootstrap profile selection are implicit
      rather than first-class.
- [ ] explain interpolation currently mixes "raw template chain" with "final
      adapted placeholder values" without making that distinction explicit.
- [ ] `alias.*` is still a magic namespace inside generic interpolation logic.
- [ ] mutation rules do not yet fully enforce the semantic difference between
      bootstrap keys and runtime keys.

### Code-shape problems

- [ ] [resolver.rs](/home/oistes/git/github.uio.no/osp/osp-cli-rust/crates/osp-config/src/resolver.rs)
      is too large at 779 lines and carries too many responsibilities.
- [ ] bootstrap, scope selection, interpolation, explain collection, and top
      level orchestration are all in one file.
- [ ] the file is readable today, but it will be hard to keep readable as new
      behavior lands.

### Product problems

- [ ] the config contract does not yet fully explain which keys are bootstrap
      inputs and which are runtime values.
- [ ] `config explain` needs a more explicit story for bootstrap-derived values.
- [ ] config mutation commands need clearer errors for invalid scopes on
      bootstrap keys.

## Design Summary

Use three phases with explicit boundaries.

Bootstrap should be expressive by layer, but intentionally restricted by scope.

That means:

- [x] bootstrap may read across all normal layers once those layers exist
- [x] bootstrap must ignore scopes that require an already-active profile
- [x] bootstrap must never recurse back into file selection or profile-scoped
      runtime resolution

Short version:

- layer-wide
- scope-restricted
- multi-phase
- non-recursive

### Phase 1: Bootstrap Paths

Choose which files to load before any file-backed config is read.

Reads only:

- CLI overrides
- env overrides
- terminal kind
- explicit app/session bootstrap inputs
- fixed OS discovery rules

Does not read:

- values from the config file being selected
- profile-scoped config
- file layers
- secrets
- normal runtime placeholder interpolation

This phase is intentionally narrow. It only decides which files and bootstrap
inputs exist before normal loading starts.

### Phase 2: Bootstrap Profile

Choose the active profile after layers are loaded, but before profile-scoped
runtime resolution.

Reads only non-profile-dependent entries from:

- explicit `--profile`
- bootstrap-visible session override
- builtin defaults
- config file
- environment
- CLI
- session
- `profile.default` from global scope
- `profile.default` from terminal-only scope
- built-in fallback `"default"`

Recommended policy:

- [x] path bootstrap reads only pre-file inputs
- [x] profile bootstrap reads across all loaded layers
- [x] profile bootstrap ignores any profile-dependent scope
- [ ] secrets do not participate in bootstrap profile selection unless we later
      have a concrete need

My recommendation is to keep secrets out of bootstrap profile selection. It is
extra complexity for little value.

Does not read:

- profile-scoped `profile.default`
- profile+terminal-scoped `profile.default`
- profile-dependent interpolation for `profile.default`
- interpolation requiring profile-scoped values

### Phase 3: Runtime Resolution

Resolve all non-bootstrap keys for one fixed `(active_profile, terminal)` frame.

Runtime pipeline:

1. select one raw winner per key by layer + scope precedence
2. interpolate placeholders over those winners
3. adapt and validate against schema
4. inject derived runtime values
5. freeze the snapshot

This keeps the current Rust philosophy and avoids the Python engine's feedback
loops.

## Bootstrap Contract

### `profile.default`

`profile.default` is a bootstrap key.

It should:

- [x] decide `profile.active` when no explicit override exists
- [x] be resolved across all loaded layers, but only from scopes that do not
      require an active profile
- [x] be absent from `ResolvedConfig.values`
- [x] not appear as a normal runtime key selected under profile scope
- [x] use bootstrap-aware explain output

It should not:

- [ ] vary by active profile
- [ ] silently disagree with `profile.active`
- [ ] behave like an ordinary profile-scoped setting

### Config path selection

Config-path selection is also bootstrap.

The sane rule is:

- [x] config file path may depend on CLI/env/session/bootstrap context
- [x] config file path may not depend on values stored in the file being chosen

This is the clean answer to "config depends on env / session values" without
making the whole resolver recursive.

Valid example:

1. env sets `OSP_CONFIG_FILE=/tmp/work.toml`
2. path bootstrap selects `/tmp/work.toml`
3. file loader reads `/tmp/work.toml`
4. profile bootstrap reads `profile.default = "ops"` from that file
5. runtime resolution proceeds with `profile.active = "ops"`

Invalid example:

1. runtime config tries to compute `config.path`
2. file selection depends on profile-scoped or interpolated runtime values
3. profile selection depends on the file chosen that way

That is exactly the recursive design this plan is trying to avoid.

## Bootstrap Key Registry

Bootstrap keys should not be modeled as scattered special cases.

Add one explicit registry or schema annotation that defines, for each
bootstrap-only key:

- [x] whether the key is bootstrap-only or runtime-visible
- [x] which bootstrap phase owns it
- [x] which scopes are valid
- [ ] which sources/layers are valid
- [ ] whether it has a derived runtime mirror

Conceptually:

- [x] `profile.default` => bootstrap-only, phase: profile bootstrap, scopes:
      global + terminal-only
- [ ] future path keys => bootstrap-only, phase: path bootstrap, scopes:
      probably global-only unless there is a real reason otherwise

This registry should become the authority for:

- validation
- mutation rules
- explain routing
- docs consistency

## TOML Rules

These should remain valid:

```toml
[default]
profile.default = "work"

[terminal.cli]
profile.default = "ops"

[profile.work]
region = "eu-north-1"

[profile.personal]
region = "local"
```

These should become hard config errors:

```toml
[profile.work]
profile.default = "personal"
```

```toml
[terminal.cli.profile.work]
profile.default = "personal"
```

If we later want more bootstrap keys, they should follow the same rule: only
global or terminal-only scopes unless explicitly documented otherwise.

## Public API Target

The current public surface is close, but it needs a cleaner bootstrap story.

Recommended additions:

```rust
struct BootstrapRequest {
    profile_override: Option<String>,
    terminal: Option<String>,
    config_file_override: Option<PathBuf>,
    secrets_file_override: Option<PathBuf>,
}

struct BootstrapPaths {
    config_file: Option<PathBuf>,
    secrets_file: Option<PathBuf>,
}

struct BootstrapProfile {
    active_profile: String,
    default_profile: Option<String>,
    source: BootstrapProfileSource,
}

struct BootstrapResult {
    paths: BootstrapPaths,
    profile: BootstrapProfile,
}
```

Recommended resolver-facing methods:

- [ ] `discover_bootstrap_paths(...)`
- [ ] `load_layers_for_bootstrap(...)`
- [ ] `resolve_bootstrap_profile(...)`
- [ ] `resolve_runtime(...)`
- [x] `explain_bootstrap_key(...)`

The point is not to add surface area for its own sake. The point is to stop
smuggling bootstrap logic through normal runtime resolution.

## `resolver.rs` Makeover

The current file should be decomposed into a small orchestration module plus a
few focused helpers.

Suggested split:

- [x] `bootstrap.rs`
      path discovery, bootstrap-profile selection, bootstrap explain types
- [x] `selector.rs`
      scope ranking and layer selection
- [x] `interpolate.rs`
      placeholder parsing, interpolation, cycle detection
- [x] `explain.rs`
      explain-layer and explain-interpolation assembly
- [x] `resolver.rs`
      small orchestration layer that wires the pieces together

Target shape:

- [x] top-level `resolver.rs` should read like orchestration, not implementation
- [ ] comments should describe policy, not compensate for tangled control flow
- [x] helper modules should each have one job

This should reduce the file from "one good but growing file" to "one readable
entrypoint and a few obvious submodules".

## Data Model Changes

### Keep

- [ ] `Scope`
- [ ] `ConfigLayer`
- [ ] `ResolvedValue`
- [ ] `ResolvedConfig`
- [ ] `ConfigExplain`

### Add

- [x] bootstrap explain types
- [x] bootstrap result types
- [x] explicit bootstrap source enum for profile selection
- [x] bootstrap key registry or schema annotation

### Adjust

- [x] stop treating `profile.default` as a required runtime-resolved key
- [x] keep `profile.active` as the required runtime key
- [x] make `ResolvedConfig` carry only runtime values

That last point matters. Runtime snapshots should not contain keys whose meaning
changes between bootstrap and runtime.

## Explain Semantics

This area needs a clearer contract than it has today.

### Runtime explain

For normal keys:

- [x] show layer candidates
- [x] show winner
- [x] show raw template when interpolation happened
- [x] show interpolation dependencies
- [x] show final runtime value

### Bootstrap explain

For bootstrap keys such as `profile.default`:

- [x] show bootstrap candidates only
- [x] show why profile scope was ignored
- [x] show the final chosen `profile.active`
- [x] show whether CLI override bypassed `profile.default`

Recommended shape:

- [x] use a separate bootstrap explain result type instead of forcing bootstrap
      and runtime explain into one structure with optional fields

### Placeholder explain rule

Make the current ambiguity explicit:

- [x] record raw placeholder source values separately from final adapted values
- [x] decide which one the UI prints by default
- [ ] do not pretend those are the same thing

Recommended direction:

- use raw values for interpolation traces
- use final values for final resolved output
- label both clearly

## Secret Handling

Secret behavior should be explicit and boring.

- [x] keep secret propagation through interpolation
- [x] guarantee explain output is redacted by default
- [x] ensure both runtime explain and bootstrap explain use the same redaction
      policy
- [ ] avoid any formatting path that accidentally reveals secret payloads

The current `SecretValue` type is a good base. The plan is to tighten usage, not
replace it.

## Placeholder Contract

Keep placeholder syntax intentionally small.

- [x] `${key}` remains the only placeholder syntax
- [x] unresolved placeholders are errors
- [x] cycles are errors
- [x] no shell-style defaults
- [x] no nested mini-language
- [x] no implicit runtime computation

If alias expansion needs different behavior, that should live in alias-specific
code, not in the generic resolver.

## Alias Policy

The resolver should not carry unexplained magic namespaces.

Recommended direction:

- [x] decide whether `alias.*` participates in generic interpolation at all
- [x] if not, document it as an explicit policy and keep alias expansion outside
      resolver internals
- [x] if yes, remove the special casing and make alias behavior follow normal
      config rules

My preference is to keep alias expansion out of the generic config resolver and
stop teaching the resolver alias-specific behavior.

## Schema And Validation

The schema story should stay stricter than the Python resolver.

- [x] validate unknown keys strictly except `extensions.*`
- [x] keep adaptation strict
- [x] do not silently keep the old value when adaptation should be a contract
      failure
- [ ] separate bootstrap validation from runtime validation where useful

Recommended change:

- [x] `profile.default` should be validated in bootstrap resolution, not as part
      of required runtime keys
- [x] keep placeholder errors runtime-specific

## Error Model

The error types should reflect the new contract.

Potential additions:

- [x] `InvalidBootstrapScope { key, scope }`
- [ ] `BootstrapKeyNotRuntimeVisible { key }`
- [ ] `InvalidBootstrapValue { key, reason }`

Potential cleanups:

- [ ] keep placeholder errors runtime-specific
- [ ] keep file-load errors independent of runtime profile selection
- [ ] keep mutation validation errors clear and user-facing

Validation timing:

- [x] reject invalid bootstrap-key scopes as early as possible
- [x] prefer load-time or layer-validation-time rejection over deferred
      resolution-time rejection

Preferred rule:

- [x] if a layer contains `profile.default` in profile or profile+terminal
      scope, loading that layer should fail

If that proves awkward in a specific path, resolution-time rejection is an
acceptable fallback, but load-time rejection is the design target.

## Loader And Runtime Pipeline

[runtime.rs](/home/oistes/git/github.uio.no/osp/osp-cli-rust/crates/osp-config/src/runtime.rs)
already has the right basic idea: discover paths first, then build a loader
pipeline.

The makeover should make that sequence first-class:

- [ ] path bootstrap
- [ ] load layers
- [ ] bootstrap profile selection
- [ ] runtime resolution

Recommended refactor in `runtime.rs`:

- [ ] separate `RuntimeConfigPaths::discover()` from profile bootstrap
- [ ] keep file discovery out of `ConfigResolver`
- [ ] make `build_runtime_pipeline(...)` accept already-resolved bootstrap paths
- [ ] thread bootstrap metadata back to callers that need explain/debug output

## CLI And Mutation Rules

These rules should be enforced consistently in CLI, store helpers, and config
commands.

- [x] `--profile` always wins
- [x] `config set profile.default ...` is allowed only in global or
      terminal-only scope
- [x] `config set --profile foo profile.default ...` is rejected
- [x] `config set --profile foo --terminal repl profile.default ...` is rejected
- [x] `config unset` follows the same scope rules
- [x] `config explain profile.active` explains runtime selection
- [x] `config explain profile.default` explains bootstrap selection

This is where breaking changes are a benefit. Silent ignore would be worse than
hard errors.

## Migration Plan

### Phase 0: freeze current behavior with tests

- [x] add tests for current `profile.default` bootstrap behavior
- [x] add tests for explain output on interpolated keys
- [x] add tests for secret redaction in explain output
- [x] add tests for alias behavior as it exists today

### Phase 1: lock bootstrap semantics

- [x] add bootstrap-key registry / validation rules
- [x] reject profile-scoped `profile.default`
- [x] reject profile+terminal-scoped `profile.default`
- [x] remove `profile.default` from normal runtime selection
- [x] add explicit bootstrap-profile resolution

### Phase 2: split the file

- [x] move scope selection into `selector.rs`
- [x] move interpolation into `interpolate.rs`
- [x] move explain assembly into `explain.rs`
- [x] move bootstrap logic into `bootstrap.rs`
- [x] shrink `resolver.rs` to orchestration

### Phase 3: clean up public contracts

- [x] introduce bootstrap result/explain types
- [x] update `ResolvedConfig` to runtime-only semantics
- [x] update config commands to enforce bootstrap-key scope rules

### Phase 4: docs and UX

- [x] update [docs/CONFIG.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/CONFIG.md)
- [x] update [docs/STATE.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/STATE.md)
- [x] update [docs/ARCHITECTURE.md](/home/oistes/git/github.uio.no/osp/osp-cli-rust/docs/ARCHITECTURE.md)
- [x] update config explain output text/JSON contract

## Test Plan

### Bootstrap profile tests

- [x] explicit profile override beats bootstrap default
- [x] terminal-only `profile.default` beats global default for that terminal
- [x] profile-scoped `profile.default` errors
- [x] profile+terminal-scoped `profile.default` errors
- [ ] empty or non-string `profile.default` errors

### Runtime resolution tests

- [ ] profile+terminal beats profile
- [ ] profile beats terminal
- [ ] terminal beats global
- [ ] later entry in same layer wins
- [ ] later layer wins across layers

### Interpolation tests

- [ ] basic substitution
- [ ] multi-hop substitution
- [ ] unresolved placeholder error
- [ ] cycle error
- [ ] secret propagation through interpolation
- [ ] explain distinguishes raw placeholder value from final adapted value

### Explain tests

- [ ] runtime explain shows candidate chain correctly
- [x] bootstrap explain shows bootstrap chain correctly
- [ ] redaction is consistent in text and JSON output

### Mutation tests

- [x] invalid scoped writes to `profile.default` fail
- [ ] valid global and terminal-only writes succeed
- [x] unset follows the same validation rules

## What We Explicitly Will Not Build

- [ ] config callables
- [ ] iterative fixed-point resolution
- [ ] loader dependency solver
- [ ] context stabilization passes
- [ ] profile-dependent config-file selection
- [ ] bootstrap keys that secretly fall back to runtime semantics

Those are exactly the kinds of features that would erase the Rust resolver's
main advantage over the Python one.

## Success Criteria

We are done when all of these are true:

- [ ] a new contributor can explain bootstrap vs runtime resolution from one doc
- [ ] [resolver.rs](/home/oistes/git/github.uio.no/osp/osp-cli-rust/crates/osp-config/src/resolver.rs)
      reads like orchestration, not a wall of implementation
- [x] `profile.default` and `profile.active` no longer have ambiguous roles
- [x] `config explain` is trustworthy for both runtime and bootstrap keys
- [ ] tests cover the semantic contract, not just the current implementation
- [ ] the Rust resolver is obviously smaller and easier to reason about than
      Python `ResolverV2`

That is the bar for the makeover.
