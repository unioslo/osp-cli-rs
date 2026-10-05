# completion

Turns a partially typed line plus a cursor position into ranked suggestions.
Pure: no terminal, no network, no plugin processes, no host state. The REPL and
`osp repl debug-complete` are its callers.

Imports only `core`. Live data (provider manifests, hostnames, netgroups)
arrives already baked into the tree; this module never fetches it.

## Layout

| File | Owns |
|---|---|
| `model.rs` | Plain data: `CompletionTree`, `CompletionNode`, `FlagNode`, `ArgNode`, `SuggestionEntry`, analysis types |
| `tree.rs` | `CommandSpec` and `CompletionTreeBuilder`; lowering `CommandDef` into nodes |
| `parse.rs` | Shell-like tokenizing and what the cursor is editing |
| `context.rs` | Resolving the command path, flag scope and selected provider |
| `suggest.rs` | Matching and ranking: exact, prefix, word-boundary, then fuzzy |
| `engine.rs` | `CompletionEngine`: parse, resolve, suggest in one call |

## Rules worth knowing

- A suggestion's `value` is what gets inserted; `display` is only a label.
- `SuggestionEntry::aliases` match typed input but are never listed alone.
- Other spellings of one flag carry `FlagNode::alias_of`; menus show the
  preferred spelling once, as `--interactive, -i`.
- Required flags come from `FlagHints` declared by the command or provider.
  Completion reports them; it never invents requiredness.
- Large catalogues use `PrefixValues`: nothing below three characters, at most
  25 prefix matches.

## Common changes

- **New kind of suggestion data:** add a field to the node in `model.rs`,
  fill it in `tree.rs` or in the caller's `augment_completion`, read it in
  `suggest.rs`.
- **Ranking change:** `match_score` and `compare_suggestions` in `suggest.rs`.
  Check `suggest/tests.rs` and the REPL contract tests.

## Debugging

```sh
osp repl debug-complete --line 'orch vm create --provider vmware --os ' --format json
```

The JSON shows the replace range, stub and every candidate in rank order.
