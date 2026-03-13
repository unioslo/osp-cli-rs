# Getting Started

This is the shortest useful path through `osp`.

The goal is simple:

1. run one command
2. start the REPL and inspect something interactively
3. know the first troubleshooting commands before you need them

If you are looking for site-specific command surfaces, read the downstream
product docs for your distribution. This file is about the generic upstream
experience.

## 1. Confirm The Binary Works

Install and inspect the top-level help:

```bash
cargo install osp-cli
osp --help
```

If you are building from source instead:

```bash
cargo install --path .
osp --help
```

## 2. Run One Useful CLI Command

Start with built-in commands that exist in every upstream install:

```bash
osp plugins list
osp plugins commands
osp plugins commands --json
```

That gives you three different answers:

- which plugins were discovered
- which command roots are currently visible
- what the command catalog looks like in machine-readable form

If you are unsure what `osp` thinks right now, also run:

```bash
osp config show
osp config explain ui.format
```

## 3. Start The REPL

Run:

```bash
osp
```

Then try:

```text
plugins list
plugins commands --format md
help config
```

That covers the core upstream promise:

- the REPL reuses the same command grammar as the CLI
- help and formatting behave the same way interactively

Use full commands first. REPL shell scope exists only for a small set of
shellable domain roots and is not part of the generic upstream quick start.

## 4. Learn The First Three Troubleshooting Commands

These are the highest-value first checks:

```bash
osp plugins doctor
osp config explain <key>
osp -d plugins list
```

Use them for:

- missing or unhealthy plugin commands
- confusing config winners
- a quick stderr-side diagnostic pass without changing stored defaults

## 5. Where To Go Next

- want copy-pasteable patterns:
  [COOKBOOK.md](COOKBOOK.md)
- want a deeper REPL guide:
  [REPL.md](REPL.md)
- want to understand config precedence:
  [CONFIG.md](CONFIG.md)
- want to debug an issue:
  [TROUBLESHOOTING.md](TROUBLESHOOTING.md)

You can ignore plugin authoring, protocol, and architecture docs until you are
extending `osp` or working on the repo.
