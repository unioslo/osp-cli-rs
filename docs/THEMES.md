# Themes

Themes control the color palette and some presentation-facing style tokens used
by `osp`.

Themes do not change command data, config precedence, or DSL behavior. They
change how the same help, tables, prompts, and message blocks look.

If you want fewer borders or less help chrome, that is mostly
[`UI.md`](UI.md). If you want different colors and prompt styling, that is
mostly this document.

## What Themes Affect

Themes feed:

- message and section colors
- prompt colors
- table/help/guide chrome tokens
- interactive selection/completion styling

Themes do not change:

- which format is chosen
- which command runs
- what rows or documents are returned

## How A Theme Is Chosen

Selection order is simple:

1. `--theme <name>` for this process
2. `theme.name` from resolved config
3. the built-in default theme

That means `--theme` is a one-shot override. It does not write config.

## Common Commands

List themes:

```bash
osp theme list
```

Inspect one theme:

```bash
osp theme show dracula
```

Switch theme for the current process or REPL session:

```bash
osp theme use dracula
```

Persist a theme as the default:

```bash
osp config set theme.name dracula --save
```

The important distinction is:

- `theme use <name>` changes the current process/session
- `config set theme.name <name> --save` changes stored config

## Built-In Themes

Current built-ins:

- `plain`
- `nord`
- `dracula`
- `gruvbox`
- `tokyonight`
- `molokai`
- `catppuccin`
- `rose-pine-moon` (default)

`plain` is the boring fallback. It is useful when you want a predictable
low-chrome color story or when you are testing presentation behavior without a
strong palette.

## Themes And Presentation

Theme and presentation are related, but not the same:

- `theme.name` chooses colors and theme-derived style tokens
- `ui.presentation` seeds a broader UI profile such as chrome density and intro
  defaults

You can mix them freely. For example:

```toml
[default]
theme.name = "dracula"
ui.presentation = "compact"
```

That gives you Dracula colors with compact layout defaults.

## Custom Themes

Custom themes live in `theme.path` directories. By default `osp` looks in the
platform config root under `osp/themes`, for example:

- `~/.config/osp/themes` on Linux when `XDG_CONFIG_HOME` is not set
- `$XDG_CONFIG_HOME/osp/themes` when `XDG_CONFIG_HOME` is set

Each `*.toml` file defines one theme.

Minimal example:

```toml
base = "dracula"

[palette]
accent = "#123456"
title = "bold #123456"
```

Rules:

- `id` and `name` are optional
  - `id` defaults to the filename stem
  - `name` defaults to a display-friendly title derived from `id`
- `base` is optional
  - if present, missing palette keys inherit from that built-in base theme
  - if omitted or set to `"none"`, missing palette keys stay unset
- color/style specs accept `#RRGGBB` and named colors
- style specs may also use prefixes such as `bold`, `dim`, `italic`, or
  `underline`

Useful config:

```toml
[default]
theme.name = "my-theme"
theme.path = ["~/.config/osp/themes", "/srv/shared/osp-themes"]
```

## Inspecting Custom Theme State

`theme list` shows whether a theme is `builtin` or `custom`, and includes the
origin path for custom themes.

`theme show` includes:

- `base`
- `source`
- `origin`
- the current palette values

That makes it the first command to run when a theme does not look the way you
expected.

## If Themes Look Wrong

Start with:

```bash
osp theme list
osp theme show <name>
osp doctor theme
osp config explain theme.name
```

That usually answers one of four questions:

- did the theme load?
- which theme actually won?
- did a custom file override a builtin?
- did the custom theme file have load issues?
