# src

The generic `osp` host: CLI and REPL, config, completion, the row DSL,
rendering, and the plugin and native-command boundaries. Nothing here knows
about UiO; site products such as `osp-cli-uio` wrap this crate and register
their own native commands.

## Layers

Lower layers must not import higher ones.

| Layer | Modules | Role |
|---|---|---|
| Primitives | `core`, `normalize` | Rows, output model, command metadata, plugin wire DTOs; identifier normalization |
| Building blocks | `config`, `completion`, `dsl`, `guide`, `ui` | Reusable on their own; no host state |
| Command providers | `plugin`, `native` | External and in-process commands as catalog metadata |
| Grammar | `cli` | What a user may type, and one-shot flags |
| Host | `app`, `repl` | Composition: load config, build the catalog, dispatch, render |

`lib.rs` re-exports the public surface; `bin/osp.rs` is a thin process entry.

## Measured imports

Non-test code, counted from `crate::` paths:

| Module | Imports |
|---|---|
| `core` | nothing |
| `config` | `core`, `normalize`, `plugin` (one timeout constant) |
| `completion` | `core` |
| `dsl` | `core`, `guide`, `ui`, `cli` (one row adapter) |
| `guide` | `core`, `ui` |
| `ui` | `core`, `config`, `guide` |
| `plugin` | `core`, `config`, `completion`, `native`, `app` (`ConfigState` only) |
| `cli` | almost everything, including `app` |
| `app`, `repl` | everything |

The three narrow exceptions are tolerated, not a pattern. Prefer moving the
shared fact down a layer over adding another upward import.

## Where to start

- A new user-visible command: `cli` for the grammar, `app/builtin.rs` for
  dispatch, or `native` when a wrapper crate owns it.
- Output looks wrong: `ui`, after checking the command produced the right
  `OutputResult`.
- `| F …` behaves oddly: `dsl`.
- Tab or the prompt behaves oddly: `repl/engine` and `completion`.
- A config key: `config`.

Each module's `mod.rs` documents its contract; the folder READMEs cover layout
and common changes.
