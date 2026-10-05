# guide

Help, intro and command reference as data. A `GuideView` holds sections,
entries and prose; other code can filter it, pipe it through the DSL, or render
it, instead of passing pre-rendered strings around.

Imports `core` (command metadata, rows) and `ui` for section chrome. Layout
decisions stay in `ui`; this module only describes content.

## Layout

| File | Owns |
|---|---|
| `mod.rs` | `GuideView` and its sections; `from_command_def`, `from_text` |
| `template.rs` | The restricted markdown used for intros and authored guides, with `osp` blocks |
| `pipeline.rs` | Guide content as ordinary rows, so `help config | F name~set` works |

## Common changes

- **Help for a command looks wrong:** usually the clap definition or its
  `CommandDef`, not this module. `from_command_def` only copies what it gets.
- **New intro placeholder:** templates are rendered with values supplied by
  `repl/presentation.rs`; add the value there, not here.

## Debugging

```sh
osp help config
osp help config --json        # the GuideView as data
```
