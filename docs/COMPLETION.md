# REPL Completion and History (Rust Plan)

This document summarizes how osprov-cli completion works today and proposes a
Rust design that keeps the sophistication but reduces complexity.

## Current Behavior (osprov-cli)

Completion is built in three explicit phases:

1. Build a completion tree from Typer metadata and config schema.
2. Parse the current input line into a command context.
3. Generate suggestions based on that context.

The completion engine is pure. It does not do IO or network calls. Dynamic
data is injected by the REPL layer.

### Completion tree

The tree is a nested map where:

- Command names map to child subtrees.
- Flags are keys like `--provider` with metadata.
- Value nodes are marked with `__value_key__` or `__value_leaf__`.
- Metadata keys are prefixed with `__`.

Key metadata used today:

- `__tooltip__` for help text.
- `__suggestions__` and `__suggestions_by_provider__`.
- `__multi__` and `__flag_only__`.
- `__type__ = "path"` for file path completion.
- `__args__` for positional argument metadata.
- `__request_hints__` for `--request key=value` completion.
- `__flag_hints__` for provider-scoped flag visibility.
- `__os_versions__` and `__os_provider_map__` for the OS two-step.

### Input parsing

- Uses shell-like parsing with `|` treated as punctuation.
- Forgives unmatched quotes while typing.
- Splits into command head, flags, args, and DSL pipes.

### Suggestion engine

- Suggests subcommands, flags, or values depending on cursor position.
- Handles DSL pipe completions when `|` is present.
- Hides already-used flags unless re-typing the same flag.
- Handles the `--os` two-step (family → version).
- Handles `--request` CSV completions with prefix preservation.
- Merges full-line flags into cursor context so later flags influence
  earlier completions.

### Dynamic augmentation

The REPL lazily injects dynamic data:

- Orchestrator capabilities.
- OpenAPI-derived `view` and `fields` lists.
- Provider OS catalogs and request hints.

This keeps the completion engine pure while still offering rich hints.

### REPL integration

- `prompt_toolkit` `ThreadedCompleter` + `FuzzyCompleter`.
- Command lexer highlights valid commands based on the tree.
- Two-layer history: prompt_toolkit history for navigation and a separate
  HistoryManager for `!!`, `!123`, `!prefix` expansion.

## Rust Design (Simplified, Still Sophisticated)

We keep the same three-phase engine but simplify the tree and metadata
to a smaller, stable schema.

### Proposed completion tree schema

- `CommandNode { children, flags, args, meta }`
- `FlagNode { name, takes_value, multi, suggestions, meta }`
- `ArgNode { suggestions, meta }`

Dynamic hints are attached at runtime by the REPL layer.

### Input parsing

Use a small parser based on `shell-words`:

- Tokenize with `|` as a separate token.
- Keep a parsed structure: `head`, `flags`, `flag_order`, `pipes`.
- Compute the cursor stub (empty stub means suggest new tokens).

### Suggestion engine

Keep these behaviors:

- Flag suggestions and value suggestions.
- Provider-scoped flags and values.
- DSL verb completion after `|`.
- OS two-step completion.
- `--request` key=value completion with prefix preservation.
- Path completion for path-typed flags.

### History and bang expansion

Implement in Rust exactly as today:

- `!!` expands to last command.
- `!-N` expands to Nth previous.
- `!123` expands to absolute id.
- `!prefix` expands to last command with that prefix.

This is independent of the line editor’s own history.

## Line Editor Choices (Prompt Toolkit Equivalent)

### Recommended: `reedline`

Pros:

- Built-in menus and multi-column completions.
- Reverse search and history menus.
- Custom prompt rendering (left and right prompts).
- Highlighters and validators.
- Completion and hint APIs are flexible.

Cons:

- Slightly heavier dependency tree.

### Alternative: `rustyline`

Pros:

- Lighter dependency tree.
- Familiar readline-like behavior.

Cons:

- Less powerful completion UI.
- More manual work for multi-column menus and metadata.

## Suggested Rust Crates

- `reedline` for the REPL line editor.
- `fuzzy-matcher` for fuzzy ranking of suggestions.
- `unicode-width` for prompt width calculations.
- `anstyle` or `nu-ansi-term` for styled prompts and hints.

## Implementation Outline

1. Build a static completion tree from the command registry and config schema.
2. Add a `CompletionAugmentor` that injects dynamic hints when needed.
3. Implement a pure `CompletionEngine` with parse + suggest stages.
4. Integrate with `reedline` `Completer` and `Highlighter` traits.
5. Add history manager with bang-expansion and prefix search.
6. Add tests for completion parity and history expansion.

## Tests to Port Early

- Command and flag name completion.
- Flag value completion from enums or literals.
- DSL verb completion after `|`.
- Provider-scoped flags and OS two-step completion.
- `!!`, `!-N`, `!123`, `!prefix` expansion.
