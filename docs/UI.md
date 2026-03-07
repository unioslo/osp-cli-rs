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

- Explicit `mode`, `color`, and `unicode` knobs.
- Presentation presets that seed a coherent UI profile without leaking preset
  names into renderer logic.
- Config schema includes all user-facing UI knobs.
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

These are the core controls:

- `ui.mode`: `auto | plain | rich`
- `ui.presentation`: `expressive | compact | austere`
- `ui.color.mode`: `auto | always | never`
- `ui.unicode.mode`: `auto | always | never`
- `theme.name`: active theme preset name
- `ui.width`: optional override for terminal width
- `ui.format`: default output format
- `ui.chrome.frame`: `none | top | bottom | top-bottom | square | round`
- `ui.table.border`: `none | square | round`
- `ui.help.layout`: `full | compact | minimal`
- `ui.messages.layout`: `grouped | minimal`
- `ui.short_list_max`, `ui.medium_list_max`, `ui.grid_padding`, `ui.grid_columns`
- `ui.table.overflow`: `clip | ellipsis | wrap | none`

### Auto behavior

- If stdout is not a TTY, default to `plain`.
- If `NO_COLOR` is set, colors are off.
- If `TERM=dumb`, unicode is off.
- If `ui.mode=plain`, color + unicode are disabled regardless of other settings.

## Presentation Presets

`ui.presentation` is a convenience preset, not a rendering backend.

- `expressive`
  - rich-friendly defaults
  - stronger section chrome
  - multiline prompt
  - full help and intro density
- `compact`
  - simpler prompt
  - lighter section chrome
  - compact help
  - grouped messages
- `austere`
  - plain rendering by default
  - no section chrome
  - square ASCII tables
  - minimal help, intro, and messages

The old `gammel-og-bitter` name is only a CLI compatibility alias. The
canonical preset name is `austere`.

## Override Precedence

For user-visible UI behavior, think in this order:

1. Invocation flags for one command:
   `--json`, `--format`, `--mode`, `--color`, `--unicode`, `-v/-q`, `-d`,
   `--plugin-provider`
2. Session/bootstrap overrides:
   `--presentation`, `--theme`, REPL session writes, launch context
3. Stored config and environment
4. `ui.presentation` seeded defaults for keys still at builtin default
5. Builtin defaults

Important rule:

- explicit per-key UI settings beat the presentation preset
- invocation flags never write back into config
- `config explain` shows when a preset seeded the effective value

## Message Blocks

Message groups on `stderr` now use the same chrome system as other structured
UI surfaces, with an explicit message layout knob:

- `ui.messages.layout`: `grouped | minimal`
- grouped messages now use the same `ui.chrome.frame` section chrome as help,
  intro, and command overviews
- divider style follows unicode/ascii mode
- divider width follows resolved output width

This keeps user-facing diagnostics visually strong without mixing message
chrome into data output.

## Help Layout

Help output has its own density knob:

- `ui.help.layout`: `full | compact | minimal`
- `ui.chrome.frame` still controls geometry
- `ui.help.layout` controls spacing and body density

This keeps help readable without overloading the chrome setting.

Presentation presets are visible in `config explain` when they materially
change an effective UI value. The raw config winner still stays visible, and
the explain output adds a `presentation` section with the preset, its source,
and the seeded effective value.

## Prompt Styling

REPL prompt visuals are config-seeded and theme-aware:

- `repl.prompt` template (`{user}`, `{domain}`, `{profile}`, `{context}`,
  `{indicator}`)
- `repl.simple_prompt`
- `repl.shell_indicator`
- `repl.intro`
- `repl.intro.style`
- `color.prompt.text`
- `color.prompt.command`
- `color.text`
- `color.key`
- `color.value`
- `color.message.error`
- `color.message.warning`
- `color.message.success`
- `color.message.info`
- `color.message.trace`

If prompt color keys are unset, semantic theme tokens are used.

Current presentation behavior:

- `expressive` keeps the multiline prompt template and full intro chrome
- `compact` uses the simple single-line prompt and minimal intro
- `austere` uses the simple single-line prompt, minimal intro, minimal help,
  and minimal message layout

## CLI Flags

These are invocation-local overrides:

- `--format {json,table,mreg,value,md,auto}`
- `--mode {plain,rich,auto}`
- `--color {auto,always,never}` and `--no-color`
- `--unicode {auto,always,never}` and `--ascii`
- `--theme <name>` remains a bootstrap/session concern

Formatting flags do not write into config state. They override the effective
render settings for the current invocation only.

## Examples

Logging-friendly plain output:

```toml
[default]
ui.mode = "plain"
ui.color.mode = "never"
ui.unicode.mode = "never"
ui.chrome.frame = "none"
ui.table.border = "square"
ui.messages.layout = "minimal"
```

Compact REPL:

```toml
[terminal.repl]
ui.presentation = "compact"
repl.simple_prompt = true
ui.help.layout = "compact"
```

Austere operator profile:

```toml
[profile.ops]
ui.presentation = "austere"
ui.table.border = "square"
ui.messages.layout = "minimal"
repl.intro.style = "minimal"
```

One-shot override without changing defaults:

```bash
osp --presentation compact ldap user alice
osp ldap user alice --json
osp repl
```

## Benchmark Target

There is now a small benchmark example for representative renderer workloads:

```bash
cargo run -p osp-ui --example render_bench --release -- 500
```

It measures:

- rich table rendering
- plain table rendering
- rich JSON rendering
- rich MREG rendering
- grouped message rendering

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
