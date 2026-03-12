# OSP Docs

Start here if you are using `osp`.

This folder is about product behavior and operator use. If you are trying to
understand code ownership or internal layering, the Rust module docs under
`src/*/mod.rs` are the better place to start.

## Architecture And Module Docs

If you want the code-level map instead of the operator guide, start with:

- crate map: [`src/lib.rs`](/home/oistes/git/github.uio.no/osp/osp-cli-rust/src/lib.rs)
- host/runtime composition: [`src/app/mod.rs`](/home/oistes/git/github.uio.no/osp/osp-cli-rust/src/app/mod.rs)
- CLI grammar: [`src/cli/mod.rs`](/home/oistes/git/github.uio.no/osp/osp-cli-rust/src/cli/mod.rs)
- config system: [`src/config/mod.rs`](/home/oistes/git/github.uio.no/osp/osp-cli-rust/src/config/mod.rs)
- DSL pipeline: [`src/dsl/mod.rs`](/home/oistes/git/github.uio.no/osp/osp-cli-rust/src/dsl/mod.rs)
- REPL boundary: [`src/repl/mod.rs`](/home/oistes/git/github.uio.no/osp/osp-cli-rust/src/repl/mod.rs)
- UI/rendering: [`src/ui/mod.rs`](/home/oistes/git/github.uio.no/osp/osp-cli-rust/src/ui/mod.rs)
- plugin boundary: [`src/plugin/mod.rs`](/home/oistes/git/github.uio.no/osp/osp-cli-rust/src/plugin/mod.rs)

## Broad-Strokes Mental Model

```text
command line
  ↓
profile + config resolution
  ↓
built-in command or plugin dispatch
  ↓
optional DSL pipeline
  ↓
UI/rendering or REPL presentation
```

Most questions about `osp` fit one of those stages. The best doc is usually
the one that owns that stage.

## Start Here By Task

- I want to run one command and control output:
  [FORMATTING.md](FORMATTING.md)
- I want to work interactively:
  [REPL.md](REPL.md)
- I want to understand config, profiles, and why one value won:
  [CONFIG.md](CONFIG.md)
- I want to change themes or presentation:
  [THEMES.md](THEMES.md) and [UI.md](UI.md)
- I want to use plugins:
  [USING_PLUGINS.md](USING_PLUGINS.md)
- I want to write a plugin:
  [WRITING_PLUGINS.md](WRITING_PLUGINS.md),
  [PLUGIN_PROTOCOL.md](PLUGIN_PROTOCOL.md), and
  [PLUGIN_PACKAGING.md](PLUGIN_PACKAGING.md)
- I want to troubleshoot odd behavior:
  [TROUBLESHOOTING.md](TROUBLESHOOTING.md)

## Other Useful References

- DSL user guide: [DSL.md](DSL.md)
- DSL author notes: [DSL_AUTHORS.md](DSL_AUTHORS.md)
- Completion and history behavior: [COMPLETION.md](COMPLETION.md)
- Rendering and UI behavior: [UI.md](UI.md)
- Logging and debug behavior: [LOGGING.md](LOGGING.md)
- Auth and command policy: [AUTH.md](AUTH.md)
- Minimal LDAP command/service surface: [LDAP.md](LDAP.md)

## Five-IQ Quick Start

Run one command and ask for JSON:

```bash
osp ldap user alice --json
```

Start the REPL:

```bash
osp
```

Fetch once in the REPL, then keep slicing locally:

```text
ldap user alice --cache | P uid mail
ldap user alice --cache | VALUE uid
```
