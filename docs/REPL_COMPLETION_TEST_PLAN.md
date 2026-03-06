# REPL Completion Test Plan

Goal: keep REPL completion correct, repeatable, and debuggable without
relying on manual “try it in a terminal” testing.

This is now a living map of the completion test surface, not just a wish
list. It records what already exists, what contract those tests pin down,
and what still looks worth adding.

## Current Shape

The completion stack is tested at three levels:

1. `osp-completion`
   Pure parse / analyze / suggest behavior.
2. `osp-repl`
   Debug harness and menu layout behavior without a PTY.
3. `osp-cli`
   Product-level wiring: scoped REPL behavior, plugin-fed completion, and a
   small PTY smoke suite.

The important architectural rule is unchanged:

- `osp-completion` owns suggestion semantics
- `osp-repl` owns menu behavior and debug instrumentation
- `osp-cli` owns command surface, scope, aliases, plugins, and REPL policy

## Stable Debug Harness Contract

Hidden command:

`osp repl debug-complete --line "<line>" --cursor <n> --width <cols> --height <rows> --menu-ansi --menu-unicode [--step <step>]...`

Supported steps:

- `tab`
- `backtab`
- `up`
- `down`
- `left`
- `right`
- `accept`
- `close`

Contract:

- stdout is JSON only
- no `--step` returns one state object
- one or more `--step` flags returns an array of `{ step, state }` frames

State fields currently pinned by tests and callers:

- `line`
- `cursor`
- `replace_range`
- `stub`
- `matches`
- `selected`
- `selected_row`
- `selected_col`
- `columns`
- `rows`
- `visible_rows`
- `menu_indent`
- `menu_description`
- `menu_description_rendered`
- `menu_styles`
- `width`
- `height`
- `unicode`
- `color`
- `rendered`

Trace support:

- `OSP_REPL_TRACE_COMPLETION=1`
- `OSP_REPL_TRACE_PATH=/path/to/trace.jsonl`

If this contract changes, tests should fail. That is intentional.

Primary files:

- [lib.rs](/home/oistes/git/github.uio.no/osp/osp-cli-rust/crates/osp-repl/src/lib.rs)
- [debug_completion.rs](/home/oistes/git/github.uio.no/osp/osp-cli-rust/crates/osp-repl/tests/debug_completion.rs)

## Coverage Matrix

### Engine / Model

Covered today:

- root command fuzzy completion
- subcommand narrowing
- late-provider merge for OS suggestions
- suppressing already-present flags before the cursor
- provider-aware flag/value filtering
- pipe verb suggestions and fuzzy pipe matching
- subcommand metadata and preview text

Primary files:

- [parity.rs](/home/oistes/git/github.uio.no/osp/osp-cli-rust/crates/osp-completion/tests/parity.rs)
- [engine.rs](/home/oistes/git/github.uio.no/osp/osp-cli-rust/crates/osp-completion/src/engine.rs)
- [suggest.rs](/home/oistes/git/github.uio.no/osp/osp-cli-rust/crates/osp-completion/src/suggest.rs)
- [parse.rs](/home/oistes/git/github.uio.no/osp/osp-cli-rust/crates/osp-completion/src/parse.rs)

### REPL Debug / Layout

Covered today:

- debug output includes menu styles and selection state
- step-wise debug flow accepts after repeated `tab`
- menu shows selected description
- description truncates on narrow width
- columns shrink on small width
- visible rows respect available height
- description is omitted when no lines are available
- ANSI and non-ANSI rendering behavior stays stable
- replace ranges are exposed through menu/debug state

Primary files:

- [debug_completion.rs](/home/oistes/git/github.uio.no/osp/osp-cli-rust/crates/osp-repl/tests/debug_completion.rs)
- [menu.rs](/home/oistes/git/github.uio.no/osp/osp-cli-rust/crates/osp-repl/src/menu.rs)
- [menu_core.rs](/home/oistes/git/github.uio.no/osp/osp-cli-rust/crates/osp-repl/src/menu_core.rs)

### CLI / Product Wiring

Covered today:

- shell-rooted completion tree follows active scope
- completion falls back to root for unknown scope
- partial root completion does not enter a shell
- scoped completion and dispatch prefixing stay aligned
- plugin-fed completion works end to end
- LDAP plugin contributes subcommands and flags through `--describe`

Primary files:

- [completion.rs](/home/oistes/git/github.uio.no/osp/osp-cli-rust/crates/osp-cli/src/repl/completion.rs)
- [surface.rs](/home/oistes/git/github.uio.no/osp/osp-cli-rust/crates/osp-cli/src/repl/surface.rs)
- [cli_ldap.rs](/home/oistes/git/github.uio.no/osp/osp-cli-rust/crates/osp-cli/tests/contracts/cli_ldap.rs)
- [app.rs](/home/oistes/git/github.uio.no/osp/osp-cli-rust/crates/osp-cli/src/app.rs)

### PTY Smoke

Covered today:

- first `tab` opens the menu
- repeated `tab` cycles selection
- accepting a completion updates the input buffer
- a single visible completion is accepted cleanly

Primary file:

- [pty_repl_completion.rs](/home/oistes/git/github.uio.no/osp/osp-cli-rust/crates/osp-cli/tests/pty_repl_completion.rs)

## Checklist

### Debug Harness

- [x] `osp repl debug-complete` returns stable JSON state
- [x] stepped debug frames are supported
- [x] trace JSONL can be enabled with env vars
- [x] menu style/debug fields are covered by tests

### Engine / Model

- [x] root command fuzzy completion
- [x] subcommand narrowing
- [x] late-provider value merge
- [x] already-present flag suppression
- [x] pipe verb suggestions
- [x] fuzzy long-verb pipe matching
- [x] provider-aware flag filtering
- [ ] explicit regression for quoted token replace ranges in completion debug output
- [ ] explicit regression for Unicode completion tokens

### REPL Layout

- [x] first `tab` does not skip the first item
- [x] description line renders for selected item
- [x] description truncates on narrow width
- [x] visible rows respect available height
- [x] ANSI/non-ANSI rendering paths are covered
- [x] rendered width stays bounded in menu tests

### CLI / Wiring

- [x] shell scope roots completion
- [x] unknown scope falls back to root
- [x] partial completion does not trigger shell entry
- [x] completion and dispatch prefixing agree in scoped shells
- [x] plugin `--describe` metadata feeds REPL completion
- [x] LDAP contract covers plugin subcommands and flags
- [ ] alias-expanded partial completion agreement test

### PTY Smoke

- [x] open menu with `tab`
- [x] move selection with repeated `tab`
- [x] accept selection and update buffer
- [ ] explicit close-menu-and-keep-typing assertion

## Determinism Rules

PTY tests should keep forcing:

- `TERM=xterm-256color`
- `COLUMNS=80`
- `LINES=24`
- `NO_COLOR=1`
- REPL intro disabled
- history disabled unless history behavior is the thing under test

PTY assertions should stay small and structural:

- sentinel substrings
- trace events
- accepted values

Avoid full-screen snapshots.

## Remaining High-Value Additions

These are still worth doing, in roughly this order:

1. Add an explicit completion debug test for quoted-token replacement ranges.
2. Add an explicit Unicode token regression in `osp-completion`.
3. Add one alias-expanded partial completion agreement test at the `osp-cli`
   layer.
4. Add one PTY assertion that closes the menu and keeps typing on the same
   line.

That should be enough. The goal is not a giant completion test framework; the
goal is a small, durable surface that catches semantic drift quickly.
