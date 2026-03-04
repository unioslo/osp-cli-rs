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

## Progress Snapshot

### Implemented in the 60% -> 80% tranche

- UI knobs are now config-driven through render settings:
  - `ui.indent`, `ui.short_list_max`, `ui.medium_list_max`
  - `ui.grid_padding`, `ui.grid_columns`, `ui.column_weight`
- Table and MREG IR now preserve raw JSON values until render time.
- Markdown renderer now supports alignment-aware separators and padded columns.
- MREG grid layout now uses column-wise fill and list-threshold decisions from
  render settings.
- Message rendering switched to `ui.messages.format` (`rules|groups|boxes`).
- Line parts can carry style tokens and are rendered accordingly in rich mode.
- Layout context now keys metrics by stable block identity (pointer id), not
  enumeration index, to avoid drift when block ordering changes.
- Introduced `osp_core::output_model` as the shared UI/DSL boundary contract
  (`OutputResult`, `OutputItems`, `Group`, `OutputMeta`).
- `osp-ui` now supports rendering directly from `OutputResult` (rows or grouped
  payloads), with row-based helpers retained as compatibility wrappers.
- Grouped table rendering now carries `header_pairs` metadata and renders
  summary pairs before each table.
- `osp-cli` now routes one-shot plugin responses and REPL command outputs
  through `OutputResult` + `render_output`, preserving metadata-driven column
  ordering where available.

### Remaining for the 80% -> 95% tranche

- Add non-default table alignment metadata plumbing from grouped payloads to
  markdown/grid renderers.
- Add richer value-style mapping from config keys (`color.value.*`,
  `color.panel.title`, etc.) instead of theme-only token mapping.
- Add doc/help panel formatting parity and richer code/JSON styling options.
