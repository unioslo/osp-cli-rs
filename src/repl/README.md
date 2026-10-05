# repl

Everything that only exists when `osp` runs interactively: the line editor,
shells such as `orch` scoped prompts, history, aliases in the live catalog,
the prompt and intro, and restarting after config changes. One-shot CLI runs
never enter this module.

It sits at the host layer and imports `app`, `cli`, `completion`, `config`,
`dsl`, `ui` and the rest. Nothing below the host layer imports `repl`.

## Layout

| Path | Owns |
|---|---|
| `engine/` | The reedline boundary: editor, menus, hinter, prompt, debug surfaces |
| `dispatch/` | Running an accepted line: builtins, shell scope, command handoff |
| `host.rs`, `lifecycle.rs` | The loop: build a cycle, run it, restart on reload |
| `surface.rs` | The browse/completion surface derived from the command catalog |
| `completion.rs` | Shaping that surface into a completion tree: aliases, invocation flags, shell re-rooting |
| `highlight.rs` | Input colouring and the first invalid word |
| `menu.rs`, `menu_core.rs` | The completion menu: reedline adapter and editor-free layout/navigation |
| `history.rs`, `history_store.rs` | History policy, `!!`/`!n` expansion, the persistent store |
| `input.rs` | Classifying a raw line before dispatch |
| `presentation.rs`, `help.rs` | Prompt, intro, REPL help, appearance from the theme |

## Common changes

- **New REPL setting:** add the default in `config/defaults.rs` and the schema
  entry in `config/core.rs`, read it in `presentation.rs`, carry it on
  `ReplRunConfig`. `repl.tab_mode` is a small worked example.
- **Builtin that only makes sense interactively:** `dispatch/builtins.rs`.
- **Completion looks wrong for a command:** first check the tree with
  `debug-complete`. If the data is right, the problem is in `engine/` or the
  menu; if not, in `completion.rs` or the command's own metadata.

## Debugging

```sh
osp repl debug-complete --line 'ldap ' --format json
osp repl debug-highlight --line 'orch vm fnd'
OSP_REPL_TRACE_COMPLETION=1 OSP_REPL_TRACE_PATH=/tmp/trace.jsonl osp
```

The trace records every menu refresh, cycle and accept with the buffer before
and after. It is the quickest way to tell a data problem from a paint problem.
