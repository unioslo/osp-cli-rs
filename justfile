set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

fmt:
    cargo fmt --all --check

fmt-fix:
    cargo fmt --all

clippy:
    ./scripts/check-rust-fast.sh

test:
    cargo test

check:
    ./scripts/check-rust-fast.sh
    cargo test

precommit:
    ./scripts/check-rust-fast.sh
