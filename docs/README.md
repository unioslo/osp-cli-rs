# OSP Docs

Start here if you are using `osp`.

If you are evaluating `osp-cli` as a library or wrapper-crate base instead of
as an operator, jump straight to [EMBEDDING.md](EMBEDDING.md) and the crate
root rustdoc in [`../src/lib.rs`](../src/lib.rs). If you want the copyable
starting point, use [`../examples/product-wrapper/src/lib.rs`](../examples/product-wrapper/src/lib.rs)
and [`../examples/product-wrapper/src/main.rs`](../examples/product-wrapper/src/main.rs).

This folder mixes operator docs, customization docs, plugin/extender docs, and
contributor notes. If you are new, stay in `First Stops` and `Using osp` first.
You can ignore `Extending osp`, `Contributor Docs`, and `Architecture And
Module Docs` until you actually need them.

## First Stops

- new here:
  [GETTING_STARTED.md](GETTING_STARTED.md)
- want copy-pasteable patterns:
  [COOKBOOK.md](COOKBOOK.md)
- something is already weird:
  [TROUBLESHOOTING.md](TROUBLESHOOTING.md)

## Common Jobs

- inspect which plugin-provided commands are available:
  [COOKBOOK.md](COOKBOOK.md) and [USING_PLUGINS.md](USING_PLUGINS.md)
- debug a missing command or provider conflict:
  [TROUBLESHOOTING.md](TROUBLESHOOTING.md) and
  [USING_PLUGINS.md](USING_PLUGINS.md)
- set sane daily defaults for output and presentation:
  [COOKBOOK.md](COOKBOOK.md), [CONFIG.md](CONFIG.md), and [UI.md](UI.md)

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

## Using osp

- guided first session:
  [GETTING_STARTED.md](GETTING_STARTED.md)
- REPL workflow:
  [REPL.md](REPL.md)
- one-shot output, flags, and rendering:
  [FORMATTING.md](FORMATTING.md)
- config, profiles, and precedence:
  [CONFIG.md](CONFIG.md)
- pipes and output shaping:
  [DSL.md](DSL.md)
- completion and history:
  [COMPLETION.md](COMPLETION.md)
- first-pass debugging:
  [TROUBLESHOOTING.md](TROUBLESHOOTING.md)

## Customizing osp

- themes:
  [THEMES.md](THEMES.md)
- UI and presentation behavior:
  [UI.md](UI.md)
- logging and debug output:
  [LOGGING.md](LOGGING.md)

## Extending osp

Skip this whole section unless you are building on top of `osp-cli`, working
with plugin-provided commands, or writing plugins.

- building a site-specific product crate on top of `osp-cli`:
  [EMBEDDING.md](EMBEDDING.md)
- using plugin-provided commands:
  [USING_PLUGINS.md](USING_PLUGINS.md)
- writing plugins:
  [WRITING_PLUGINS.md](WRITING_PLUGINS.md)
- subprocess protocol:
  [PLUGIN_PROTOCOL.md](PLUGIN_PROTOCOL.md)
- bundled plugin packaging:
  [PLUGIN_PACKAGING.md](PLUGIN_PACKAGING.md)
- auth and command policy boundary:
  [AUTH.md](AUTH.md)
- site-specific integrations belong in downstream product repositories

## Contributor Docs

- contributing workflow:
  [../CONTRIBUTING.md](../CONTRIBUTING.md)
- testing strategy and confidence lanes:
  [TESTING.md](TESTING.md)
- DSL implementation notes:
  [DSL_AUTHORS.md](DSL_AUTHORS.md)
- planning/review scratch area:
  `docs/plans/`

## Architecture And Module Docs

If you want the code-level map instead of the operator guide, start with these.
Most users can ignore this section.

For rendered API docs, prefer docs.rs or `cargo doc --open`. The links below
point to the corresponding source entrypoints in this repository.

- crate map: [`src/lib.rs`](../src/lib.rs)
- host/runtime composition: [`src/app/mod.rs`](../src/app/mod.rs)
- CLI grammar: [`src/cli/mod.rs`](../src/cli/mod.rs)
- config system: [`src/config/mod.rs`](../src/config/mod.rs)
- DSL pipeline: [`src/dsl/mod.rs`](../src/dsl/mod.rs)
- REPL boundary: [`src/repl/mod.rs`](../src/repl/mod.rs)
- UI/rendering: [`src/ui/mod.rs`](../src/ui/mod.rs)
- plugin boundary: [`src/plugin/mod.rs`](../src/plugin/mod.rs)
