# UI Parity Spec (Rust)

This spec defines the expected behavior for `osp-ui` while migrating from
`osprov-cli`.

## Scope

- Keep the UI layer pure:
  - formatters convert data to IR
  - layout prepass computes widths/alignment
  - renderers produce output only
- Keep stdout/stderr contract unchanged:
  - data documents on stdout
  - grouped messages on stderr

## Formats

Supported formats:
- `json`
- `table`
- `md`
- `mreg`
- `value`
- `auto`

`auto` selection rules:
- single-column `value` rows -> `value`
- single row (non-value) -> `mreg`
- multi-row -> `table`

## IR Requirements

Minimum IR block set:
- `Line`
- `Panel`
- `Code`
- `Json`
- `Table`
- `Value`
- `Mreg`

The IR must not contain command/plugin-specific semantics.

## Layout Requirements

A prepass must build a `LayoutContext` before rendering.

Current invariants:
- table column widths are shared globally by header key
- table widths shrink-to-fit by reducing the widest participating column
- mreg metrics include key-width and available content width

## MREG Requirements

MREG list rendering uses semantic thresholds:
- short lists render inline (`a, b, c`)
- medium lists render vertical
- long lists render grid

MREG scalar lines align keys with a common key width.

## Markdown Requirements

- escape `\` and `|`
- keep deterministic header/row structure
- respect precomputed widths for readable alignment

## Copy Requirements

- copy rendering always uses plain backend
- no ANSI sequences in copy output
- no unicode border dependency in copy output

## Acceptance Tests

The following must stay green:
- `crates/osp-ui/src/layout.rs` tests for layout invariants
- `crates/osp-ui/src/renderer.rs` tests for:
  - mreg list behavior
  - markdown/table rendering
  - width truncation
  - color/unicode toggles

