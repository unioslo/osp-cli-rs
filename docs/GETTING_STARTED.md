# Getting Started

This guide walks through a first session with `osp`:

1. run one command
2. start the REPL and inspect something interactively
3. know the first troubleshooting commands before you need them

For site-specific commands, use the product documentation for your
distribution. These examples use commands included in the upstream install.

## 1. Confirm The Binary Works

Install and inspect the top-level help:

```bash
cargo install osp-cli --locked
osp --help
```

If you are building from source instead:

```bash
cargo install --path . --locked
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

To inspect the active configuration, run:

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

The REPL uses the same commands, help, and formatting options as the CLI.

Use full commands first. REPL shell scope exists only for a small set of
shellable domain roots. When those roots are available, typing the bare root
enters that shell. It is not part of the generic upstream quick start.

## 4. Learn The First Three Troubleshooting Commands

Start with these commands when something goes wrong:

```bash
osp plugins doctor
osp config explain <key>
osp -d plugins list
```

Use them for:

- missing or unhealthy plugin commands
- unexpected configuration values
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
