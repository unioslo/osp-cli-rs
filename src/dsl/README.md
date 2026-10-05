# dsl

The row pipeline after `|`: `orch task find | F status=running | S -created_at | L 10`.
A command produces rows; this module filters, projects, sorts, groups and
limits them. The same stages work in the CLI, the REPL, and on JSON piped in
from a file.

Imports `core` for rows and output, `guide` and `ui` for in-band help (`| H`),
and one `cli` row adapter in `value.rs`. It never runs commands or reads config.

## Layout

| Path | Owns |
|---|---|
| `parse/` | Splitting `command | stages`, lexing stages, key paths and quick search |
| `model.rs` | Parsed stage data |
| `compiled.rs` | Turning a parsed stage into a plan, one arm per verb |
| `engine.rs` | Running plans over rows and group metadata |
| `eval/` | Field resolution, flattening and matchers shared by verbs |
| `verbs/` | One file per verb: `filter`, `project`, `sort`, `group`, `aggregate`, `jq`, … |
| `verb_info.rs` | The registry: verb spelling, summary, help text with an example |
| `value.rs` | JSON fixtures entering through the same adapter as plugin output |

## Rules worth knowing

- Verbs see canonical rows. Display formatting happens later, in `ui`.
- Group keys and aggregates are metadata on the result; member rows stay intact.
- Unknown fields in a non-empty result are errors, not silent empty columns.

## Adding a verb

1. Write `verbs/<name>.rs` with a `compile(spec)` function and the apply
   function the engine will call; `filter.rs` and `limit.rs` show the usual
   shapes for rows and groups.
2. Add the arm in `compiled.rs` and the execution arm in `engine.rs`.
3. Register it in `verb_info.rs` with a copyable example; REPL completion and
   `| H` read this list, so nothing else needs updating for discovery.
4. Document it in `docs/DSL.md`.

## Debugging

```sh
osp plugins commands | H            # verb list
osp plugins commands | H F          # one verb's syntax
osp plugins commands --json | P name about
```
