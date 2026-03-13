# Cookbook

This file is a grab bag of small, copyable patterns.

Unless noted otherwise, every example here uses upstream command surfaces that
exist in a plain `osp-cli` install.

Examples that use `inventory ...` are illustrative provider-backed command
shapes. Replace them with a real plugin command from `osp plugins commands`.

## Inspect Plugin-Provided Commands

```bash
osp plugins list
osp plugins commands
```

## Ask For JSON Once

```bash
osp plugins commands --json
```

## Force Plain JSON For Scripts

```bash
osp --format json --mode plain plugins commands
```

## Explain Why A Config Value Won

```bash
osp config explain ui.format
osp config get ui.format --sources
```

## Show The Whole Resolved Config

```bash
osp config show
osp config show --sources
```

## Start A Short REPL Session

```text
plugins list
plugins commands --format md
help config
```

## Ask For Help In Guide Form

```bash
osp help --guide
```

```text
help config
```

## Switch Output Format For One Command

```bash
osp plugins commands --format md
osp plugins commands --value
```

## Pick One Provider For One Invocation

Illustrative provider-backed example:

```bash
osp inventory host web-01 --plugin-provider inventory-a
```

## Debug A Provider Conflict

Illustrative provider-backed example:

```bash
osp plugins commands
osp plugins doctor
osp inventory host web-01 --plugin-provider inventory-a
osp plugins select-provider inventory inventory-a
```

## Cache One Provider Result In The REPL

Illustrative provider-backed example:

```text
inventory host web-01 --cache | P name owner
inventory host web-01 --cache --format json
```

## Set Sane Daily Defaults

```bash
osp config set ui.presentation compact --save
osp config set ui.format table --save
osp config set repl.simple_prompt true --save
```

## Do A First-Pass Plugin Health Check

```bash
osp plugins doctor
osp -d plugins doctor
```

## Keep Reading

- guided first session:
  [GETTING_STARTED.md](GETTING_STARTED.md)
- REPL behavior:
  [REPL.md](REPL.md)
- formatting and output:
  [FORMATTING.md](FORMATTING.md)
- troubleshooting:
  [TROUBLESHOOTING.md](TROUBLESHOOTING.md)
