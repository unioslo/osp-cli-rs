# Theme System

This document captures what to bring from `osprov-cli` theming and what to
leave behind.

## What `osprov-cli` Does Well

- Semantic token model (`palette.*`, `color.*`) instead of command-specific
  hardcoded colors.
- Built-in theme catalog with recognizable presets.
- Runtime theme switching for preview.
- REPL prompt styling connected to the active palette.

## What Was Messy In `osprov-cli`

- Theme command internals import private preset data directly.
- Theme data is discovered by resolver spelunking instead of using a dedicated
  typed registry.
- Heavy runtime mutation/patching around Typer/Rich style globals.
- Theme management and config mutation logic are tightly coupled.

## Rust Direction

- Keep a typed, explicit theme registry in `osp-ui`:
  `crates/osp-ui/src/theme.rs`.
- Keep semantic style tokens in one place:
  `crates/osp-ui/src/style.rs`.
- Keep rendering decisions in UI only; no DSL/dispatch logic in UI.
- Seed theme from config and allow CLI override.

## Implemented Behavior

- Global option: `--theme <name>`
- Commands:
  - `osp theme list`
  - `osp theme show [name]`
  - `osp theme use <name>` (current process/session)
- REPL builtins:
  - `theme list`
  - `theme show [name]`
  - `theme use <name>`
- Config seeding:
  - `theme.name` is read from resolved config when `--theme` is not passed.
- REPL prompt styling:
  - `repl.prompt`, `repl.simple_prompt`, `repl.shell_indicator`, `repl.intro`
  - `color.prompt.text`, `color.prompt.command`
- Message blocks:
  - themed divider/rule rendering with `ui.messages.format`
- Theme palette inspection:
  - hex palette values render in truecolor when color is enabled

## Current Theme Catalog

- `plain`
- `nord`
- `dracula`
- `gruvbox`
- `tokyonight`
- `molokai`
- `catppuccin`
- `rose-pine-moon` (default)

## Current Gaps

- Persistent write path for `theme.name` waits on `config set` completion.
- User-defined theme files (theme directory loader) are not implemented yet.
