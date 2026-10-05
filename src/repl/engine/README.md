# repl/engine

The boundary with reedline. Everything that needs reedline types lives here or
in `../menu.rs`; the rest of `repl` talks in lines and results.

## Layout

| File | Owns |
|---|---|
| `session.rs` | Building the `Reedline` editor, keybindings, the read loop |
| `editor.rs` | `AutoCompleteEmacs` edit mode, the prompt, terminal capability probing |
| `adapter.rs` | `ReplCompleter` and the highlighter/history adapters |
| `hint.rs` | Muted inline suggestion plus the status line under the prompt |
| `overlay.rs` | Menu construction and the Ctrl-R history picker (skim) |
| `config.rs` | Public run config: `ReplRunConfig`, `ReplAppearance`, `ReplTabMode` |
| `debug.rs` | The `debug-complete` and menu debug surfaces |

## How a key travels

1. reedline reads a key; `AutoCompleteEmacs::parse_event` turns it into an
   event. Menu navigation becomes a plain buffer edit here, before painting.
2. reedline applies the event. Tab on a closed menu reaches
   `OspCompletionMenu::complete_like_shell` through partial completion.
3. reedline paints: highlighter, then the hinter, then an open menu. The hinter
   records the painted line in `PaintedLine` for the next key.

## Constraints that shape the code

- reedline only lends a menu the editor while painting, after the line is
  drawn. Anything that changes the buffer must happen at key time, or the line
  lags one frame behind the menu.
- An edit mode never sees the buffer. `PaintedLine` is the workaround; it can
  be one paint stale when keys arrive in a batch, as with a paste.
- reedline hides hints while a menu is open and on submit, so the status line
  never reaches scrollback.
- Without colour there is no hint style, so no inline suggestion; it would
  look like typed text.

## Common changes

- **Key binding:** `build_repl_keybindings` in `session.rs`.
- **When typing opens the menu:** `AutoCompleteEmacs::opens_menu`.
- **What the status line says:** `ReplHinter::status`.
