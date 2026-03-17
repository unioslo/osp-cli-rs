# UI and Rendering

This document is about product behavior, not internal Rust API shape.

If you want to understand the UI pipeline as code, start with
`src/ui/mod.rs`. If you want to understand what users see, how `osp`
chooses formats, and which config knobs matter, start here.

## What the UI layer does

At a high level, rendering happens in this order:

1. a command produces structured output
2. an optional DSL pipeline rewrites that output
3. `osp` chooses an output format
4. `osp` chooses a plain or rich backend
5. the final text is rendered to stdout

The important thing to keep in mind is that `osp` does not render directly
from raw command data. It first decides what kind of output the result should
be, then renders that shape consistently.

## Format Selection

Supported formats:

- `auto`
- `guide`
- `json`
- `table`
- `md`
- `mreg`
- `value`

Selection order is:

1. an explicit `--format` or `ui.format`
2. a command/plugin recommendation attached to the output
3. automatic inference from the output shape

When `format=auto`, the current rules are:

- grouped output renders as `table`
- rows where every row only has a `value` field render as `value`
- zero or one ordinary row renders as `mreg`
- larger row sets render as `table`

`guide` is a special-purpose format for semantic help/intro output. In
practice you usually see it when a command explicitly produces guide-style
content or when you ask for it directly.

## Render Modes

Render mode controls the backend:

- `plain`: ASCII-safe, no ANSI color, no Unicode box drawing
- `rich`: rich terminal rendering, color and Unicode when allowed
- `auto`: choose based on runtime conditions

The current auto rule is intentionally boring:

- non-TTY output falls back to `plain`
- a `dumb` terminal falls back to `plain`
- otherwise `auto` uses `rich`
- forcing `color=always` or `unicode=always` keeps the rich backend active

Plain mode is strict. Once plain mode wins, color and Unicode are both off
even if the terminal could support them.

## Color, Unicode, and Width

Useful one-shot flags:

- `--format`
- `--mode`
- `--color {auto,always,never}`
- `--no-color`
- `--unicode {auto,always,never}`
- `--ascii`

Useful persistent config keys:

- `ui.format`
- `ui.mode`
- `ui.color.mode`
- `ui.unicode.mode`
- `ui.width`

`ui.width` overrides detected terminal width. This mainly matters for tables,
MREG-style layouts, and guide/help rendering where line wrapping and column
packing change visibly.

## Tables and MREG Layout

Table-oriented tuning lives here:

- `ui.table.border`
  - `none | square | round`
- `ui.table.overflow`
  - `clip | ellipsis | wrap | none`

`table` is the dense grid view for many rows.

`mreg` is the more semantic key/value view that `osp` prefers for one-row
results. It is easier to scan for a single object, while `table` is better
for comparing many rows.

## Presentation Presets

`ui.presentation` seeds a coherent UI profile:

- `expressive`
- `compact`
- `austere`

`gammel-og-bitter` remains a compatibility alias for `austere`.

These presets do not hard-lock the UI. They seed canonical keys that still
sit on builtin defaults. Once you set an explicit value like `ui.mode` or
`ui.chrome.frame`, that explicit value wins over the preset.

In practice:

- `expressive` favors richer chrome and fuller help/intro behavior
- `compact` keeps the UI denser without going fully austere
- `austere` strips the UI toward quieter, plainer output

## Help, Messages, and Section Chrome

The main keys here are:

- `ui.help.level`
  - `inherit | none | tiny | normal | verbose`
- `ui.messages.layout`
  - `grouped | plain | minimal`
- `ui.chrome.frame`
  - `none | top | bottom | top-bottom | square | round`
- `ui.chrome.rule_policy`
  - `per-section | shared`

What these do:

- `ui.help.level` controls how much help/detail is shown
- `ui.messages.layout` controls whether messages are grouped into sections or
  shown as plain indented groups or inline with minimal chrome
- `ui.chrome.frame` controls the section framing style used by help, guide, and
  grouped messages
- `ui.chrome.rule_policy` controls whether section rules are rendered
  independently or shared across sibling sections

These settings change presentation only. They do not change the underlying
command data.

## Prompt and REPL Presentation

The REPL prompt and intro surface are controlled primarily by:

- `repl.prompt`
- `repl.simple_prompt`
- `repl.shell_indicator`
- `repl.intro`
  - `none | minimal | compact | full`
- `color.prompt.text`
- `color.prompt.command`

The important distinction is:

- `repl.simple_prompt` changes prompt density
- `repl.intro` changes how much startup/help material the REPL shows
- the `color.prompt.*` keys tune prompt styling rather than general table/help
  styling

## Clipboard and Copy Mode

Clipboard copy is opt-in:

- `--copy` on supported commands
- DSL `| Y`

Copy rendering always uses the plain-safe path even when visible output is
rich. That is deliberate. Clipboard content should be stable, pasteable text,
not ANSI-decorated terminal output.

## Practical Examples

Compact REPL defaults:

```toml
[terminal.repl]
ui.presentation = "compact"
repl.simple_prompt = true
repl.intro = "compact"
```

Quiet operator profile:

```toml
[profile.ops]
ui.presentation = "austere"
ui.mode = "plain"
ui.color.mode = "never"
ui.chrome.frame = "none"
ui.messages.layout = "minimal"
```

Force Markdown for one command without changing config:

```bash
osp --format md plugins commands
```

Force plain JSON for scripting:

```bash
osp --format json --mode plain plugins list
```
