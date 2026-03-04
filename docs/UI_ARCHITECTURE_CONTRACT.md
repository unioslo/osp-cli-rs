# UI Architecture Contract

This document defines hard UI ownership boundaries for `osp-cli-rust`.
It is based on lessons from `osprov-cli`: keep the good parts (format/render
separation), remove the risky parts (UI executing command semantics).

## Goals

- Keep output quality on par with `osprov-cli`.
- Keep UI behavior deterministic and testable.
- Keep domain logic and DSL execution outside the UI crate.
- Keep one rendering contract for one-shot and REPL.

## Non-Goals

- No TUI framework.
- No UI-side command dispatch.
- No decorator-style runtime signature mutation.
- No hidden global side channels for render decisions.

## Ownership Rules

### `osp-cli` / `osp-services` own

- command parsing and dispatch
- plugin execution
- DSL parsing + execution (`P`, `F`, `V`)
- config resolution and runtime context selection

### `osp-ui` owns

- output format selection (`auto/json/table/mreg/value`)
- UI intermediate representation (document blocks)
- rendering backends (plain + rich-compatible path)
- message grouping by semantic level (`error/warning/success/info/trace`)

### Forbidden

- `osp-ui` must not import `osp-dsl`, `osp-services`, `osp-api`, or plugin code.
- `osp-ui` must not read process-wide mutable state to decide behavior.
- `osp-ui` must not execute command/domain logic.

## IR Contract

`osp-ui` IR is structural only. It represents how to render data, never command
semantics.

Current block family:

- `Block::Line { parts(tokenized text) }`
- `Block::Panel { title, body, rules, style tokens }`
- `Block::Code { code, language }`
- `Block::Json { payload }`
- `Block::Table { style, headers, rows, header_pairs, align, depth }`
- `Block::Value { values }`
- `Block::Mreg { rows(entries...) }`

Forbidden IR shape:

- command-aware blocks (e.g. `Block::CommandResult`, `Block::LdapUser`)
- profile/plugin/auth/runtime identity in block payload
- behavior flags that change dispatch semantics

## Data Flow Contract

1. Command returns response payload.
2. Caller converts payload to `osp_core::output_model::OutputResult` (or rows
   as a temporary compatibility path).
3. Caller checks command DSL capability metadata.
4. Caller applies DSL pipeline when capability allows it.
5. Caller passes normalized output + `RenderSettings` into `osp-ui`.
6. `osp-ui` builds document blocks.
7. Renderer emits terminal text.

`osp-ui` only sees rows/scalars and render settings; it does not know command
names, plugins, profiles, or auth state.

`auto` format selection must be pure: it may use only payload shape and render
settings, never command/plugin identity.

## Runtime Knobs

- `ui.render.mode`: `auto | plain | rich`
- `ui.color.mode`: `auto | always | never`
- `ui.unicode.mode`: `auto | always | never`
- `ui.verbosity.level`: semantic message visibility threshold

Behavior rules:

- `stdout` = data render output.
- `stderr` = grouped messages + diagnostics.
- `-v/-q` change message verbosity only.
- `-d/-dd/-ddd` change developer log verbosity only.

## Render Modes

`plain`:

- no ANSI colors
- ASCII-safe output only
- deterministic fallback for non-TTY pipelines

`rich`:

- semantic styling (colors, emphasis)
- unicode borders when enabled
- no command-specific rendering branches

`auto`:

- select `rich` on TTY
- select `plain` on non-TTY

## Plugin Output Contract

When plugin responses are rendered through `osp-ui`:

- `--format json`: `stdout` is strict JSON payload render only
- non-JSON formats: `stdout` is rendered data only
- grouped messages/diagnostics always go to `stderr`

## Extension Rules

Adding a format:

1. Add block/formatter mapping in `osp-ui`.
2. Add renderer support (or explicit rejection).
3. Add contract tests for auto-selection and explicit format.

Adding styling:

1. Keep style tokens at render edge.
2. Do not leak style concerns into row transformation logic.

## Test Contract

Required tests in `osp-ui`:

- format auto-selection (`value`, `mreg`, `table`)
- unicode toggle behavior for table borders
- no ANSI escape sequences when color is disabled
- message grouping visibility thresholds

Required tests in `osp-cli`:

- stdout/stderr separation
- REPL and one-shot use the same UI pipeline contract

Boundary verification:

- `cargo tree -p osp-ui` must not include forbidden internal crates
  (`osp-dsl`, `osp-services`, `osp-api`, `osp-cli`, plugin crates)

## Migration Notes from `osprov-cli`

Keep:

- block/IR approach
- plain vs rich rendering split
- centralized formatter selection

Do not carry over:

- UI-side DSL execution
- runtime decorator mutation
- global mutable formatting side channels
- monkeypatching CLI framework internals for style behavior
