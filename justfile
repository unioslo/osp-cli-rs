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
    cargo test --manifest-path foundation/Cargo.toml --all-features --locked

workspace-test:
    cargo test --workspace --all-features --locked

cov:
    cargo llvm-cov --workspace --all-features --summary-only

cov-gate:
    ./scripts/check-coverage-gate.py

cov-gate-fast:
    ./scripts/check-coverage-gate.py --fast

cov-baseline:
    ./scripts/update-coverage-baseline.py

check:
    ./scripts/check-rust-fast.sh
    cargo test --manifest-path foundation/Cargo.toml --all-features --locked

foundation-check:
    python3 ./scripts/check-foundation-sync.py
    cargo check --manifest-path foundation/Cargo.toml --all-features --locked
    cargo clippy --manifest-path foundation/Cargo.toml --all-features --all-targets -- -D warnings
    cargo test --manifest-path foundation/Cargo.toml --all-features --locked

precommit:
    ./scripts/check-rust-fast.sh

bump target='patch' message='':
    if [[ -n "{{message}}" ]]; then \
      python3 ./scripts/bump-version.py "{{target}}" -m "{{message}}"; \
    else \
      python3 ./scripts/bump-version.py "{{target}}"; \
    fi

bump-dry target='patch' message='':
    if [[ -n "{{message}}" ]]; then \
      python3 ./scripts/bump-version.py "{{target}}" --dry-run -m "{{message}}"; \
    else \
      python3 ./scripts/bump-version.py "{{target}}" --dry-run; \
    fi

release-notes:
    python3 ./scripts/check-release-readiness.py

release-tag:
    python3 ./scripts/push-release-tag.py

release-tag-sign:
    python3 ./scripts/push-release-tag.py --sign

release *args:
    python3 ./scripts/push-release-tag.py {{args}}

release-dry *args:
    python3 ./scripts/push-release-tag.py --dry-run {{args}}

release-sign *args:
    python3 ./scripts/push-release-tag.py --sign {{args}}

verify-full:
    ./scripts/check-rust-fast.sh
    just foundation-check
    cargo test --workspace --all-features --locked
    ./scripts/check-coverage-gate.py

release-check:
    python3 ./scripts/check-release-readiness.py
    ./scripts/check-rust-fast.sh
    just foundation-check
    cargo test --workspace --all-features --locked
    ./scripts/check-coverage-gate.py
