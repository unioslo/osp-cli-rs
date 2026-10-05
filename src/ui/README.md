# ui

Turns structured output into text for a person or a pipe. Commands and the DSL
decide *what* the data is; this module decides *how it looks*: table, key/value,
guide, markdown or JSON, with theme, width, colour and unicode policy applied.

Imports `core`, `config` (render settings) and `guide`. It does not re-run
commands, re-resolve config, or invent data. If a value is wrong here, it was
wrong before it arrived.

## Flow

```text
OutputResult / GuideView
   ▼ plan/     choose the effective format and settings
   ▼ lower.rs  convert to one document IR (doc.rs)
   ▼ emit/     write terminal, markdown or JSON text
```

## Layout

| Path | Owns |
|---|---|
| `plan/` | Picking the format from the payload, flags and config |
| `doc.rs`, `lower.rs` | The document IR and lowering into it |
| `emit/` | One emitter per shape: `table`, `key_value`, `grid`, `markdown`, `json`, `terminal` |
| `settings/` | `RenderSettings` and resolving them from config |
| `theme/`, `theme_catalog.rs` | Builtin themes; loading custom theme files |
| `style/` | Style tokens mapped to theme colours |
| `messages/` | Buffered success/warning/error messages and their layout |
| `section_chrome.rs`, `chrome/` | Section rules and frames shared by help, guides and messages |
| `clipboard/` | Copy-to-clipboard transport |
| `prompt.rs` | Plain line input for command prompts and the basic REPL |

## Common changes

- **A column renders badly:** start in the emitter for that shape, then
  `lower.rs`. Check with `--json` that the data itself is right.
- **New style role:** add the token in `style/`, map it in each builtin theme
  in `theme/`, document it in `docs/THEMES.md`.
- **New output format:** a planner case, an emitter, and the `--format` value
  in `core/output.rs`.

## Debugging

```sh
osp theme list
osp plugins commands --format table --plain     # no colour, ASCII chrome
osp plugins commands --json                     # the data before ui
```
