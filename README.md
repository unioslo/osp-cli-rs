[![Verify](https://github.com/unioslo/osp-cli-rs/actions/workflows/verify.yml/badge.svg?branch=main)](https://github.com/unioslo/osp-cli-rs/actions/workflows/verify.yml)
[![Release](https://github.com/unioslo/osp-cli-rs/actions/workflows/release.yml/badge.svg)](https://github.com/unioslo/osp-cli-rs/actions/workflows/release.yml)
[![Crates.io](https://img.shields.io/crates/v/osp-cli.svg)](https://crates.io/crates/osp-cli)
[![Docs.rs](https://img.shields.io/docsrs/osp-cli)](https://docs.rs/osp-cli)

# osp-cli

<img src="docs/assets/osp-cli.png" alt="osp-cli screenshot" width="960" />

`osp-cli` is a Rust command-line tool and interactive REPL for working with
structured data.

You can also use it as a library to build a CLI with your own commands,
defaults, and integrations.

It combines:
- command execution
- interactive history, completion, and help
- layered configuration
- multiple render modes and output formats
- a small pipeline DSL
- external command plugins

Use it as:
- a normal command-line tool
- a long-running REPL with history, completion, inline help, and cached
  results
- a library for a site-specific CLI

## As A Library

To build your own CLI with `osp-cli`, start with:

- [docs/EMBEDDING.md](docs/EMBEDDING.md)
- [docs/README.md](docs/README.md)
- [`src/lib.rs`](src/lib.rs)

## Install

From crates.io:

```bash
cargo install osp-cli --locked
```

From source:

```bash
cargo install --path . --locked
```

Run it:

```bash
osp --help
osp
```

## Quick Start

CLI:

```bash
osp config show
osp theme list
osp plugins list
```

REPL:

```text
osp> plugins commands
osp> plugins commands | P name about
osp> config explain ui.format
osp> help config
```

Per-invocation flags work the same in the CLI and REPL:

```bash
osp plugins commands --json
osp plugins commands --format table -v
```

```text
plugins commands --json
plugins commands --format table -v
```

## Capabilities

- CLI and REPL entrypoints with shared command semantics
- history, completion, highlighting, and scoped shells in the REPL
- invocation-local output and debug controls
- output formats including table, JSON, markdown, mreg, and value
- a row-oriented pipeline DSL for filtering, projection, grouping,
  sorting, aggregation, and quick search
- profile-aware config with file, env, secrets, CLI, and REPL-session
  layering
- theming, color policy, unicode policy, and presentation presets
- plugin discovery and dispatch through a JSON subprocess protocol

## Configuration And Output

Default paths:

- config: `<platform-config-dir>/osp/config.toml` (for example
  `~/.config/osp/config.toml` on Linux)
- secrets: `<platform-config-dir>/osp/secrets.toml` (for example
  `~/.config/osp/secrets.toml` on Linux)

On Linux, `XDG_CONFIG_HOME` overrides the base config directory when set.

Invocation flags such as `--json`, `--format`, `--color`,
`--plugin-provider`, `-v`, `-q`, and `-d` affect only the current
command. They do not mutate stored config.

See:
- [docs/CONFIG.md](docs/CONFIG.md)
- [docs/FORMATTING.md](docs/FORMATTING.md)
- [docs/REPL.md](docs/REPL.md)

## Plugins

`osp-cli` can discover external commands from configured plugin
directories and invoke them through a documented JSON protocol.

See:
- [docs/USING_PLUGINS.md](docs/USING_PLUGINS.md)
- [docs/WRITING_PLUGINS.md](docs/WRITING_PLUGINS.md)
- [docs/PLUGIN_PROTOCOL.md](docs/PLUGIN_PROTOCOL.md)
- [docs/AUTH.md](docs/AUTH.md)

## Documentation

Start with:
- [docs/README.md](docs/README.md)

If you are just using `osp`, read these first:
- [docs/GETTING_STARTED.md](docs/GETTING_STARTED.md)
- [docs/COOKBOOK.md](docs/COOKBOOK.md)
- [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md)

Core operator guides:
- [docs/REPL.md](docs/REPL.md)
- [docs/CONFIG.md](docs/CONFIG.md)
- [docs/FORMATTING.md](docs/FORMATTING.md)
- [docs/DSL.md](docs/DSL.md)
- [docs/USING_PLUGINS.md](docs/USING_PLUGINS.md)

If you are building a product wrapper on top of `osp-cli`, read these:
- [docs/EMBEDDING.md](docs/EMBEDDING.md)

If you are writing plugin executables or other extension-side integrations,
read these:
- [docs/WRITING_PLUGINS.md](docs/WRITING_PLUGINS.md)
- [docs/PLUGIN_PROTOCOL.md](docs/PLUGIN_PROTOCOL.md)

If you are working on the repo itself, read:
- [CONTRIBUTING.md](CONTRIBUTING.md)

## Development

Useful commands:

```bash
python3 scripts/confidence.py --list
python3 scripts/confidence.py local
```

See:
- [CONTRIBUTING.md](CONTRIBUTING.md)
- [docs/TESTING.md](docs/TESTING.md)

## License

[GPLv3](LICENSE)
