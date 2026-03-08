# UI and Rendering

This document describes the user-visible rendering behavior of `osp`.

## Rendering Pipeline

Output is processed in this order:

1. the command returns data
2. an optional DSL pipeline transforms that data
3. an output format is chosen
4. plain or rich rendering is selected
5. the final text is printed

## Formats

Supported formats:

- `auto`
- `json`
- `table`
- `mreg`
- `value`
- `md`

`auto` chooses a format based on the shape of the result:

- single-row structured data usually becomes `mreg`
- value-only data becomes `value`
- wider row sets usually become `table`

## Render Modes

- `plain`: ASCII-safe, no color
- `rich`: color and unicode when allowed
- `auto`: chooses based on terminal capabilities

Non-TTY output defaults to plain rendering.

## Color and Unicode

Relevant flags:

- `--color {auto,always,never}`
- `--no-color`
- `--unicode {auto,always,never}`
- `--ascii`

Relevant config keys:

- `ui.color.mode`
- `ui.unicode.mode`

If `ui.mode=plain`, color and unicode are disabled regardless of other
settings.

## Width and Tables

`osp` respects terminal width when rendering tables.

`ui.table.overflow` controls how wide cells behave:

- `clip`
- `ellipsis`
- `wrap`
- `none`

`ui.width` can be used to override detected terminal width.

## Presentation Presets

`ui.presentation` seeds a coherent UI profile:

- `expressive`
- `compact`
- `austere`

`gammel-og-bitter` remains a compatibility alias for `austere`.

Explicit UI keys still win over the preset.

## Message and Help Layout

Relevant config keys:

- `ui.messages.layout`
- `ui.help.layout`
- `ui.chrome.frame`

These affect the presentation of help, grouped messages, and other structured
UI surfaces. They do not change machine-readable data output.

## Prompt Styling

The REPL prompt is controlled by:

- `repl.prompt`
- `repl.simple_prompt`
- `repl.shell_indicator`
- `repl.intro`
  - `none | minimal | compact | full`
- `color.prompt.text`
- `color.prompt.command`

## Invocation Flags

Useful one-shot rendering flags:

- `--format`
- `--mode`
- `--color`
- `--unicode`
- `--ascii`

These affect only the current invocation and do not write back into config.

## Example Config

Compact REPL:

```toml
[terminal.repl]
ui.presentation = "compact"
repl.simple_prompt = true
ui.help.layout = "compact"
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

## Clipboard

Clipboard copy is opt-in:

- `--copy` on commands
- DSL `| Y`

Copy uses plain rendering even when the visible output is rich.
