# config

Answers three questions the same way everywhere: which keys are legal, which
source wins, and which file edits are allowed. Runtime resolution, `config
explain` and `config set --save` all go through the same code, so they cannot
disagree.

Imports `core` and `normalize`, plus one plugin timeout constant in
`defaults.rs`. It does not render anything and does not know about the REPL.

## Layout

| File | Owns |
|---|---|
| `core.rs` | Keys, values, layers, scopes, and the schema of builtin keys |
| `defaults.rs` | Builtin default values |
| `loader.rs` | Source loaders (file, env `OSP__…`, secrets, in-memory) and the pipeline |
| `bootstrap.rs` | Choosing the active profile and terminal once per resolution |
| `selector.rs` | Scope precedence: profile+terminal > profile > terminal > global |
| `resolver.rs` | Picking a winner per key and applying the schema |
| `interpolate.rs` | `${…}` placeholders, after winners are chosen |
| `explain.rs` | The trace behind `config explain` |
| `runtime.rs` | Path discovery and the smaller `RuntimeConfig` view the app uses |
| `store.rs` | Editing TOML files in place with the same scope rules |
| `secrets.rs` | Secret backends |

Product crates put their keys under `extensions.<product>.*` and use this
module as is. They do not keep a parallel config model.

## Common changes

**Add a builtin key**

1. Default value in `defaults.rs` (`LiteralDefault`).
2. Schema entry in `core.rs` (`insert_builtin_schema_key`). Use
   `with_allowed_values` when the set is closed; `config set <key> ⇥` then
   completes the values.
3. Read it where it is used, through the resolved config.
4. Document it in `docs/CONFIG.md` or the owning area's doc.

**Change precedence:** only in `selector.rs` or `resolver.rs`, and check that
`config explain` still tells the same story.

## Debugging

```sh
osp config explain repl.tab_mode
osp config show --sources
OSP__REPL__TAB_MODE=always osp      # one-off env override
```
