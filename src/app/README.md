# app

The host. It loads config, builds the command catalog from builtins, native
commands and plugins, decides who handles a command, runs it, applies the
pipeline and renders the result. `App` and `App::builder` are the public
entry points; a product crate builds an `App` with its own defaults and native
commands.

This is the composition layer, so it may import anything in the crate. Lower
modules should not import `app`; the one current exception is
`plugin/config.rs` reading `ConfigState`.

## Layout

| File | Owns |
|---|---|
| `mod.rs`, `host.rs` | `App`, `AppBuilder`, the top-level run path, builtin `CMD_…` names |
| `bootstrap.rs`, `assembly.rs` | Startup: config, logging, catalog assembly |
| `runtime.rs`, `session.rs` | State that lives for the process vs for one logical run |
| `rebuild.rs`, `repl_lifecycle.rs` | Rebuilding state after config or auth changes |
| `dispatch.rs` | Choosing builtin, native or plugin for a command line |
| `builtin.rs` | Running builtins after an access check |
| `external.rs` | Native and plugin commands, including help and progress |
| `command_output.rs` | Shaping the final result before rendering |
| `help.rs`, `config_explain.rs` | Help rendering policy; `config explain` output |
| `access_recovery.rs` | Hooks a wrapper uses to recover from missing access (e.g. log in) |
| `facts.rs` | Pure facts derived from host state, shared by several surfaces |
| `sink.rs`, `timing.rs`, `logging.rs` | Output sinks, timing badges, developer logging |

## Rules worth knowing

- Visibility and access are decided once, from command policy, before
  dispatch. Handlers do not re-check them their own way.
- Output leaves `app` through a `UiSink`. Tests use `BufferedUiSink` to capture
  one-shot output; REPL prompts still use the real terminal.
- Site-specific auth and integrations belong in the wrapper crate, not here.

## Common changes

- **Wrapper needs a new hook:** prefer a small trait or builder option here
  over the wrapper reaching into internals; `access_recovery.rs` is the model.
- **Command chosen wrongly:** `dispatch.rs`, then `osp plugins doctor` for
  provider conflicts.
