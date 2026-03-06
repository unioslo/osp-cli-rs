# UI and Rendering (Rust Plan)

This document is grounded in how osprov-cli renders output today and proposes
a Rust design that preserves the pipeline while simplifying configuration and
making key knobs explicit (color, unicode, width).

Hard ownership/import rules are defined in `docs/UI_ARCHITECTURE_CONTRACT.md`.
This file focuses on rendering design and behavior parity.

## Current Pipeline (Python)

Data flows through fixed stages:

1. Command returns raw data (list/dict/scalar).
2. DSL pipeline (optional) transforms the data.
3. Caller normalizes output into `osp_core::output_model::OutputResult`.
4. Formatter selection (auto or explicit format).
5. Intermediate Representation (IR) document.
6. Renderer materializes IR as plain text or rich output.
7. Optional clipboard copy uses plain rendering.

The IR is a stable contract between formatters and renderers.

## What We Keep

- IR-based separation between formatting and rendering.
- Format selection (`auto`, `json`, `table`, `mreg`, `value`, `md`).
- Plain vs rich render modes.
- DSL is executed before formatting.
- Clipboard copy uses plain rendering for predictable results.


## MVP Visual Parity

The first working version must look as good as osprov-cli for LDAP output.
That means:

- Auto format picks MREG layout for single rows.
- Table layout matches column ordering and width heuristics.
- Rich output defaults on TTY with colors enabled.
- Plain output matches when `--mode plain` or `NO_COLOR` is set.

This is non-negotiable for the MVP.

## What We Improve

- Explicit `color` and `unicode` knobs (auto/on/off).
- Remove ambiguous `AppInterfaceMode` values and replace with:
  - `ui.render.mode` (plain/rich/auto)
  - `ui.color.mode` (auto/always/never)
  - `ui.unicode.mode` (auto/always/never)
- Config schema includes all UI knobs (no hidden keys).
- Output choices are decided once in a central dispatcher, not in each command.

## Proposed IR (Rust)

Define minimal, stable blocks to decouple formatting from rendering:

- `Line { parts: Vec<(String, StyleToken)>, depth }`
- `Table { headers, rows, header_pairs, align, style, depth }`
- `JsonBlock { payload: serde_json::Value }`
- `Panel { title, body, rules, border_style, title_style }`
- `CodeBlock { code, language }`
- `Mreg { rows(entries...) }` with scalar/list rendering strategies

The `Document = Vec<Block>` stays stable across renderers.

## Formatter Selection (Auto)

Mirror existing behavior:

- `json` -> `JsonBlock`
- `md` -> `Table` with markdown style
- `value` -> `Line` list of values
- `mreg` -> `MregDocument` -> semantic blocks
- `table` -> standard tables
- `auto` -> values if value-only, mreg for single row, table otherwise
- grouped payloads (`OutputItems::Groups`) render as:
  - structured JSON (`groups`, `aggregates`, `rows`) for `json`
  - one table per group with `header_pairs` for `table`/`md`
  - merged rows for `mreg`/`value`

Auto selection must stay pure: only payload shape + render settings are inputs.
No command-specific branches in UI selection.

## Rendering Backends

### Plain Renderer

- Strict fallback renderer: no ANSI colors and ASCII-only output.
- Used when mode is explicitly `plain` or when `auto` resolves to non-TTY.
- Markdown tables output unchanged.
- Used for clipboard copy and non-TTY.

### Rich Renderer

- Uses terminal styles (color + unicode).
- Applies style tokens to semantic parts (headers, MREG keys, message groups).
- Renders JSON with syntax highlighting (optional).

### Width Behavior

- Renderer accepts optional width from render settings.
- If width is set (or discovered from `COLUMNS`), wide table cells are truncated.
- Truncation is display-width aware and uses `…`/`...` based on unicode mode.
- `ui.table.overflow` controls table behavior when content exceeds width:
  - `clip`: truncate cells without suffix
  - `ellipsis`: truncate with `…` / `...`
  - `wrap`: wrap cells to multiple lines
  - `none`: disable shrink-to-fit (no truncation or wrapping)

## Config Knobs (Explicit)

These are the core controls and should be part of schema:

- `ui.render.mode`: `auto | plain | rich`
- `ui.color.mode`: `auto | always | never`
- `ui.unicode.mode`: `auto | always | never`
- `theme.name`: active theme preset name
- `ui.ascii_borders`: legacy alias for `ui.unicode.mode = never`
- `ui.width`: optional override for terminal width
- `ui.format`: default output format
- `ui.short_list_max`, `ui.medium_list_max`, `ui.grid_padding`, `ui.grid_columns`
- `ui.table.overflow`: `clip | ellipsis | wrap | none`

### Auto behavior

- If stdout is not a TTY, default to `plain`.
- If `NO_COLOR` is set, colors are off.
- If `TERM=dumb`, unicode is off.
- If `ui.render.mode=plain`, color + unicode are disabled regardless of other settings.

## Message Blocks

Message groups on `stderr` now use themed section dividers with explicit
message formatting modes:

- `ui.messages.format`: `rules | groups | boxes` (default: `rules`)
- `boxes` is accepted as a config alias and currently maps to `rules`
- divider style follows unicode/ascii mode
- divider width follows resolved output width

This keeps user-facing diagnostics visually strong without mixing message
chrome into data output.

## Prompt Styling

REPL prompt visuals are config-seeded and theme-aware:

- `repl.prompt` template (`{user}`, `{domain}`, `{profile}`, `{context}`,
  `{indicator}`)
- `repl.simple_prompt`
- `repl.shell_indicator`
- `repl.intro`
- `color.prompt.text`
- `color.prompt.command`

If prompt color keys are unset, semantic theme tokens are used.

## CLI Flags

These map directly to in-memory session config overrides:

- `--format {json,table,mreg,value,md,auto}`
- `--mode {plain,rich,auto}`
- `--color {auto,always,never}` and `--no-color`
- `--unicode {auto,always,never}` and `--ascii`
- `--theme <name>`

Command-local flags are still more specific than the session default for that
invocation. In config introspection these launch defaults may show up as
`source=session`.

## Clipboard

Clipboard copy is opt-in via:

- `--copy` on commands
- DSL `| Y` stage (sets `wants_copy`)

Copy uses plain rendering, even when the user sees rich output.

## Suggested Rust Crates

Keep it minimal and composable:

- CLI: `clap`
- Terminal detection: `is-terminal` or `crossterm`
- Styling: `anstyle` + `anstream` (or `owo-colors` for simple styling)
- Tables: `tabled` or `comfy-table`
- JSON: `serde` + `serde_json`
- Syntax highlight (optional): `syntect`
- Clipboard (optional): `arboard`
- Width calculation: `unicode-width`

Avoid heavy UI frameworks unless we need full TUI.

## Migration Plan (UI)

Phase 1: Plain Rendering
- Implement IR, formatters, and plain renderer.
- Support `--format` and `--mode`.
- Implement `ui.color.mode` + `ui.unicode.mode`.

Phase 2: Rich Rendering
- Add style tokens and palette mapping.
- Render tables and panels with colors.
- Add optional syntax highlighting for JSON and code.

Phase 3: UX Parity
- Match MREG layout behavior.
- Add clipboard integration and DSL copy flag.
- Add Markdown table rendering.

## Tests to Add Early

- Formatting selection for auto/json/table/value/mreg.
- Color/unicode toggles (snapshot-style tests).
- Plain renderer output for a minimal dataset.
- Rich renderer output under `NO_COLOR` and `TERM=dumb`.
