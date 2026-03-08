# osp-cli-foundation

`osp-cli-foundation` is the staged single-crate foundation for the future
`osp-cli-rust` package layout.

It currently mirrors the multi-crate workspace, but with a curated public
surface under top-level modules like:

- `app`
- `runtime`
- `config`
- `core`
- `dsl`
- `ui`
- `repl`
- `completion`

The internal `osp_*` modules still exist as transition shims while the
single-crate layout is being cleaned up.

This crate is generated from the workspace by
`scripts/build-foundation-crate.py`, and is currently used as the transition
staging area for eventually making the single-crate architecture canonical.

## Current status

- buildable as a standalone crate
- tested in CI
- kept in sync with the workspace by parity checks

## Running it locally

```bash
cargo run --manifest-path foundation/Cargo.toml -- --help
cargo run --manifest-path foundation/Cargo.toml
```

## Verifying the foundation crate

```bash
cargo check --manifest-path foundation/Cargo.toml --all-features --locked
cargo clippy --manifest-path foundation/Cargo.toml --all-features --all-targets -- -D warnings
cargo test --manifest-path foundation/Cargo.toml --all-features --locked
```
