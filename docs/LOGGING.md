# Logging, Verbosity, and Debugging

`osp` separates user-facing message detail from developer-facing debug output.

## User-Facing Verbosity

Use:

- `-v`
- `-vv`
- `-vvv`
- `-q`
- `-qq`

Behavior:

- default: errors, warnings, and normal success messages
- `-v`: more detail plus cause/context chain on failures
- `-vv`: rich diagnostic failure output, with snippets when available
- `-vvv`: rich diagnostic failure output plus stack backtrace
- `-q`: fewer non-essential messages
- `-qq`: quiet except for errors

These flags work in both CLI and REPL and apply to one invocation at a time.

Important split:

- `-v/-vv/-vvv` change how failures are rendered
- `-d/-dd/-ddd` change developer logs and timing

They are related but not interchangeable. A command can have a terse failure
and high debug logging, or a forensic failure render without extra debug logs.

## Developer Debug Output

Use:

- `-d`
- `-dd`
- `-ddd`

Behavior:

- `-d`: high-level debug information
- `-dd`: more detailed debug output
- `-ddd`: the most detailed terminal debug stream

Debug logs are written to `stderr`, not `stdout`.

## Timing Hints

At debug levels, `osp` shows timing information:

- CLI: timing footer on `stderr`
- REPL: right-hand prompt badge with the last command timing

Color bands:

- `0-250ms`: green
- `250-1000ms`: warning
- `>1000ms`: red

Higher debug levels may show more detailed timing breakdowns.

## REPL Behavior

In the REPL, `-v/-q/-d` are command-local. They do not permanently change the
shell’s behavior unless you change config defaults.

```text
plugins commands -v
plugins commands -vv
plugins commands -dd
```

## Config Keys

Useful config keys:

- `ui.verbosity.level`
- `debug.level`
- `log.file.enabled`
- `log.file.path`
- `log.file.level`

Use `config explain <key>` to see where an effective value comes from.

If you need the full colored forensic error render, prefer invocation flags
such as `-vvv`. That is intentionally a per-command decision rather than a
sticky logging default.

## Plugin Runtime Hints

`osp` passes logging-related hints to plugins:

- `OSP_UI_VERBOSITY`
- `OSP_DEBUG_LEVEL`

Plugins should keep data output on `stdout` and diagnostics on `stderr`.

## Scripting Safety

`--format json` stays safe for scripts even when `-d` is enabled because data
stays on `stdout` and diagnostics stay on `stderr`.

```bash
osp plugins commands --json -d 2>debug.log
```
