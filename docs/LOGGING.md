# Logging and Verbosity Migration (osprov-cli -> osp-cli-rust)

This document captures how logging and verbosity work in `osprov-cli` today,
what should be moved to Rust, and what should be improved.

## Current Behavior in `osprov-cli` (Python)

### User-facing verbosity (`-v` / `-q`)

Source:
- `osprov-cli/src/osprov_cli/app.py`
- `osprov-cli/src/osprov_cli/core/verbosity.py`
- `osprov-cli/src/osprov_cli/errors.py`

Behavior:
- Global callback maps `-v/-q` into `ui.verbosity.level`.
- Per-command wrapper stores command-local verbosity delta in `ContextVar`.
- Error renderer combines base level + delta to decide detail level.

In Rust we keep the same intent with explicit message levels:

- `Error`
- `Warning`
- `Success` (default baseline)
- `Info` (`-v`)
- `Trace` (`-vv`)

Message blocks are grouped by level at render time. This keeps call sites
simple and allows future styling (for example boxed warnings/errors) without
changing command logic.

### Developer debug (`-d`)

Source:
- `osprov-cli/src/osprov_cli/log.py`
- `osprov-cli/src/osprov_cli/cli/decorators.py`

Behavior:
- `-d/-dd/-ddd` maps to `INFO/DEBUG/TRACE` for debug sink.
- Debug override is context-local (`ContextVar`).
- File logging stays on regardless of `-d` (controlled by config level).
- Stdout sink is filtered by debug level.

### Logger initialization

Source:
- `osprov-cli/src/osprov_cli/state.py`
- `osprov-cli/src/osprov_cli/log.py`
- `osprov-cli/src/osprov_cli/cfg/defaults.py`
- `osprov-cli/src/osprov_cli/cfg/schema.py`

Behavior:
- Logger is initialized after config resolution.
- Global context (`ctx`, `terminal`, `user`) is bound once.
- File sink writes JSON-lines style records.

## What To Keep

- Separate concerns:
  - user output verbosity (`-v/-q`)
  - developer diagnostics (`-d`)
- Persistent file logs for post-mortem diagnostics.
- Scoped context for logs (profile, terminal, user, command).
- Secret redaction before writing structured logs.
- Explicit config keys for log defaults.

## What To Fix in Rust

- Do not emit developer logs to `stdout`; use `stderr`.
- Do not hand-roll per-record file writes; use non-blocking appenders.
- Do not rely on undocumented config keys (for example rotation/retention).
- Do not flatten structured fields into message strings.
- Do not depend on runtime signature mutation/decorators for verbosity flags.
- Do not drop bootstrap logs before logger setup.

## Rust Target Design

Use `tracing` stack:
- `tracing`
- `tracing-subscriber`
- `tracing-appender`

### Model

- `ui.verbosity.level`: controls error/help/detail rendering only.
- `debug.level`: controls developer logs shown on `stderr`.
- `log.file.*`: controls persistent file logging.

### CLI flags

- `-v/-vv/-vvv` and `-q/-qq` for user-facing output detail.
- `-d/-dd/-ddd` for developer diagnostics.
- Optional overrides:
  - `--log-level <warn|info|debug|trace>`
  - `--log-dir <path>`

### Recommended mapping

- User message visibility:
  - default -> `Error + Warning + Success`
  - `-v` -> add `Info`
  - `-vv` -> add `Trace`
  - `-q` -> hide `Success`
  - `-qq` -> hide `Success + Warning` (errors still visible)
- Developer log mapping (`-d`):
  - none -> no developer log stream on terminal by default
  - `-d` -> `info`
  - `-dd` -> `debug`
  - `-ddd+` -> `trace`
- Keep `RUST_LOG` support for advanced users; CLI flag should win.

## Config Keys to Add in `osp-config`

- `ui.verbosity.level` (enum or bounded integer)
- `log.file.enabled` (bool)
- `log.file.path` (string/path)
- `log.file.level` (`warn|info|debug|trace`)
- `log.file.format` (`json|text`)
- `log.stderr.level` (optional default when `-d` is not used)

Keep all keys in schema and surfaced through `config show/get/explain`.

## Implementation Plan

### Step 1: Schema + defaults

- Add logging + verbosity keys in `osp-config` schema/defaults.
- Add adaptation and allowed-value validation.
- Add contract tests for precedence and value adaptation.

### Step 2: CLI surface

- Add global `-v/-q/-d` in `crates/osp-cli/src/cli.rs`.
- Compute effective UI verbosity and debug level once in app startup.
- Ensure both one-shot and REPL share identical semantics.

### Step 3: Logging bootstrap

- Add `crates/osp-cli/src/logging.rs` with `init_logging(...)`.
- Initialize tracing early with safe bootstrap defaults.
- Reconfigure/augment with resolved config (file path/level) once config is loaded.

### Step 4: Structured context

- Attach profile, terminal, user, command, plugin_id as structured fields/spans.
- Ensure plugin dispatch logs include plugin executable + exit status.

### Step 5: Error rendering parity

- Port existing verbosity behavior for known vs unknown errors.
- Keep concise default errors; expand details at higher verbosity.
- Ensure `--format json` output remains clean (logs must stay on stderr).

### Step 6: Contracts and integration tests

- `-d` writes debug output to stderr only.
- `--format json` stdout remains valid JSON even with `-d`.
- File logs are written at configured path and level.
- REPL command debug levels do not leak across commands.
- Redaction test for sensitive keys in structured fields.

## Why This Is Better Than `osprov-cli`

- Strong typed config contract (no hidden keys).
- Proper structured logs end-to-end (no message flattening).
- Correct stream separation (data on stdout, diagnostics on stderr).
- Real non-blocking logging pipeline and reliable file output.
- Cleaner CLI model without decorator/runtime signature mutation.
- Easier tests: deterministic levels, deterministic sinks, deterministic context.
