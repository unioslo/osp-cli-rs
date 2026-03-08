# osp-cli

[![Verify](https://github.com/unioslo/osp-cli-rs/actions/workflows/verify.yml/badge.svg?branch=main)](https://github.com/unioslo/osp-cli-rs/actions/workflows/verify.yml)
[![Release](https://github.com/unioslo/osp-cli-rs/actions/workflows/release.yml/badge.svg)](https://github.com/unioslo/osp-cli-rs/actions/workflows/release.yml)
[![Crates.io](https://img.shields.io/crates/v/osp-cli.svg)](https://crates.io/crates/osp-cli)
[![Docs.rs](https://img.shields.io/docsrs/osp-cli)](https://docs.rs/osp-cli)

`osp-cli` is a batteries-included Rust CLI and interactive REPL for
structured command workflows. It combines a command runner, completion,
history, inline help, layered configuration, themes, a small pipeline
DSL, and a plugin protocol for external commands.

`OSP` is just the project name here. The crate is not tied to one
particular data domain.

## Status

Published crate and active development target. The root package is the
canonical single-crate implementation.

## Quick start

```bash
cargo run -- --help
cargo run -- config show
cargo run -- theme list
cargo run -- plugins list
```

Start the REPL:

```bash
cargo run
```

## Features

- CLI commands and an interactive REPL with history, completion, inline
  help, and scoped shells
- Pipeline DSL for filtering, projecting, grouping, sorting, and
  aggregating row data (`| F uid=alice | P uid cn mail | S cn`)
- Multiple output formats: table, JSON, markdown, mreg, value
- Config files with profile and terminal scoping, environment variable
  overrides, and secrets handling
- Plugin system: external commands discovered and invoked via a JSON
  subprocess protocol
- Theming with color and unicode control
- Per-invocation output and debug controls shared between CLI and REPL

## Installation

### From source

```bash
cargo build --release
cp target/release/osp ~/.local/bin/
```

Bundled plugins (if present) go alongside the binary:

```
~/.local/bin/osp
~/.local/lib/osp/plugins/
  manifest.toml
  osp-uio-ldap
  osp-uio-mreg
```

## Usage

### CLI

```bash
# Config management
cargo run -- config show
cargo run -- config get ui.color.mode
cargo run -- config set ui.color.mode always
cargo run -- config explain ui.color.mode

# Plugins
cargo run -- plugins list
cargo run -- plugins commands
cargo run -- doctor
```

### REPL

When started without a command, `osp` starts an interactive REPL:

```
$ cargo run
osp> ldap user alice
osp> ldap user alice | P uid cn mail
osp> config show
osp> theme list
osp> exit
```

The REPL shares the same command grammar as the CLI. It adds history,
tab completion, syntax highlighting, inline help, and scoped shells.

### Pipeline DSL

Pipelines transform command output using `|`-separated stages:

| Verb | Purpose | Example |
|------|---------|---------|
| `P`  | Project columns | `P uid cn mail` |
| `F`  | Filter rows | `F uid=alice` |
| `S`  | Sort | `S cn` |
| `G`  | Group | `G department` |
| `A`  | Aggregate | `A count` |
| `L`  | Limit | `L 10` |
| `V`  | Extract values | `V uid` |
| `K`  | Extract keys | `K` |
| `?`  | Quick search | `? alice` |

Bare terms without a verb act as quick search. Quoting rules follow
shell conventions (`"..."` for spaces, `\|` for literal pipes).

See [docs/DSL.md](docs/DSL.md) for the full reference.

## Configuration

Config file: `~/.config/osp/config.toml`
Secrets file: `~/.config/osp/secrets.toml` (owner-only mode is enforced on Unix)

### Precedence (highest first)

Stored config resolution uses:

1. REPL session overrides
2. Environment variables (`OSP__<KEY>`)
3. Secrets file / `OSP_SECRET__<KEY>` env vars
4. Config file
5. Built-in defaults

Invocation flags such as `--json`, `--format`, `--color`, and
`--plugin-provider` apply only to the current command and do not become
stored config values.

### Profiles

Config supports profile and terminal scoping:

```toml
[default]
ui.color.mode = "auto"

[profile.prod]
ui.color.mode = "never"

[profile.prod.terminal.kitty]
ui.unicode.mode = "always"
```

Select a profile with `--profile prod`.

See [docs/CONFIG.md](docs/CONFIG.md) for details.

## Plugins

External commands are discovered from explicit plugin directories,
`OSP_PLUGIN_PATH`, bundled locations, and the user plugin directory by
default. `PATH` discovery is opt-in via
`extensions.plugins.discovery.path = true`. Plugins communicate via a
JSON-over-stdin/stdout protocol, declare their commands with
`--describe`, and receive invocations as subprocess calls.

See [docs/WRITING_PLUGINS.md](docs/WRITING_PLUGINS.md) for a guide to
writing and packaging plugins.

## Documentation

- User guide index: [docs/README.md](docs/README.md)
- Config and profiles: [docs/CONFIG.md](docs/CONFIG.md)
- REPL usage: [docs/REPL.md](docs/REPL.md)
- Output and invocation flags: [docs/FORMATTING.md](docs/FORMATTING.md)
- Plugins: [docs/USING_PLUGINS.md](docs/USING_PLUGINS.md)
- Writing plugins: [docs/WRITING_PLUGINS.md](docs/WRITING_PLUGINS.md)
- Troubleshooting: [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md)

## Development

### Build and test

```bash
./scripts/check-rust-fast.sh
cargo test --all-features --locked
```

### Run locally

```bash
cargo run                         # Start REPL
cargo run -- ldap user alice      # Single command
cargo run -- --debug ldap user x
```

### Project layout

```
src/               Canonical single-crate implementation
tests/             Integration, contract, and PTY tests
docs/              User and contributor documentation
workspace/         Legacy compatibility mirror during the transition
```

## Contributing

Contributions are welcome.

Commit messages follow `<type>(<scope>): <subject>` convention.
See [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md) for details.

## License

[GPLv3](LICENSE)
