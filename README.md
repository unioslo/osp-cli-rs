# osp-cli-rust

A Rust command-line tool and interactive REPL for querying and managing
OSP infrastructure data, with structured output, config layering, and
plugin support.

## Status

Early development. Internal tool for UiO OSP operators.
Linux is the primary platform. Rust 2024 edition (nightly may be
required).

## Quick start

```bash
cargo run -- --help
cargo run -- ldap user alice
cargo run -- ldap user alice --format json
cargo run -- ldap user alice | P uid cn mail
```

Start the REPL:

```bash
cargo run
```

## Features

- CLI commands and an interactive REPL with history, completion, and
  inline help
- Pipeline DSL for filtering, projecting, grouping, sorting, and
  aggregating row data (`| F uid=alice | P uid cn mail | S cn`)
- Multiple output formats: table, JSON, markdown, mreg, value
- Config files with profile and terminal scoping, environment variable
  overrides, and secrets handling
- Plugin system: external commands discovered and invoked via a JSON
  subprocess protocol
- Theming with color and unicode control

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
# Query LDAP
cargo run -- ldap user alice
cargo run -- ldap group staff

# Pipeline: filter, project, sort
cargo run -- ldap user alice | P uid cn mail
cargo run -- ldap users --group staff | F uid=oistes | P uid cn

# Output formats
cargo run -- ldap user alice --format json
cargo run -- ldap user alice --format table
cargo run -- ldap user alice --format value

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

When started without a command, `osp` drops into an interactive REPL:

```
$ cargo run
osp> ldap user alice
osp> ldap user alice | P uid cn mail
osp> config show
osp> theme list
osp> exit
```

The REPL shares the same command grammar as the CLI. It adds history,
tab completion, syntax highlighting, and scoped shells.

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
Secrets file: `~/.config/osp/secrets.toml` (must be mode 0600)

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

## Architecture

The root package is now the canonical single-crate implementation. The old
multi-crate layout lives under [workspace/](/home/oistes/git/github.uio.no/osp/osp-cli-rust/workspace)
as a compatibility/source mirror during the transition.

The legacy workspace still contains ten crates with a strict downward dependency graph:

```
osp-cli          Binary, app wiring, dispatch, plugin manager
osp-repl         Generic REPL engine (reedline). No business logic.
osp-completion   Tab completion tree. Cursor-aware, independent of clap.
osp-ui           Output rendering. Pure: rows in, string out.
osp-services     Business logic orchestration. No CLI/REPL imports.
osp-dsl          Pipeline DSL: parse, eval, stages. Pure transforms.
osp-config       Config loading, interpolation, scoping, secrets.
osp-ports        Trait definitions for external services (LdapDirectory).
osp-api          Mock implementations for testing.
osp-core         Domain types and enums. Zero internal dependencies.
```

Business logic (`osp-services`, `osp-dsl`) can run without the CLI or
REPL. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full
picture.

## Plugins

External commands are discovered from explicit plugin directories,
`OSP_PLUGIN_PATH`, bundled locations, and the user plugin directory by
default. `PATH` discovery is opt-in via
`extensions.plugins.discovery.path = true`. Plugins communicate via a
JSON-over-stdin/stdout protocol, declare their commands with
`--describe`, and receive invocations as subprocess calls.

See [docs/WRITING_PLUGINS.md](docs/WRITING_PLUGINS.md) for a guide to
writing and packaging plugins.

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
src/              Canonical single-crate implementation
tests/            Root package integration/contract/e2e tests
workspace/        Legacy multi-crate compatibility mirror
docs/              Architecture docs, specs, plans
```

## Contributing

Internal project. Contributions welcome from UiO OSP team members.

Commit messages follow `<type>(<scope>): <subject>` convention.
See [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md) for details.

## License

[GPLv3](LICENSE)
