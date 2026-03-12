set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

fmt:
    cargo fmt --all --check

fmt-fix:
    cargo fmt --all

clippy:
    cargo clippy --all-features --all-targets -- -D clippy::collapsible_else_if -D clippy::collapsible_if -D clippy::derivable_impls -D clippy::get_first -D clippy::io_other_error -D clippy::lines_filter_map_ok -D clippy::manual_pattern_char_comparison -D clippy::match_like_matches_macro -D clippy::needless_as_bytes -D clippy::needless_borrow -D clippy::question_mark -D clippy::redundant_closure -D clippy::unnecessary_lazy_evaluations

test:
    cargo test --all-features --locked

confidence lane='local':
    python3 ./scripts/confidence.py {{lane}}

confidence-static:
    python3 ./scripts/confidence.py static

confidence-local:
    python3 ./scripts/confidence.py local

confidence-behavior:
    python3 ./scripts/confidence.py behavior

confidence-full:
    python3 ./scripts/confidence.py full

confidence-pre-push:
    python3 ./scripts/confidence.py pre-push

cov:
    python3 ./scripts/coverage.py run --all-features --summary-only

cov-gate:
    python3 ./scripts/coverage.py gate

cov-gate-fast:
    python3 ./scripts/coverage.py gate --fast

cov-baseline:
    python3 ./scripts/coverage.py baseline

check:
    python3 ./scripts/confidence.py local

precommit:
    python3 ./scripts/public-docs.py --staged
    python3 ./scripts/confidence.py static

bump target='patch' message='':
    if [[ -n "{{message}}" ]]; then \
      python3 ./scripts/release.py bump "{{target}}" -m "{{message}}"; \
    else \
      python3 ./scripts/release.py bump "{{target}}"; \
    fi

bump-dry target='patch' message='':
    if [[ -n "{{message}}" ]]; then \
      python3 ./scripts/release.py bump "{{target}}" --dry-run -m "{{message}}"; \
    else \
      python3 ./scripts/release.py bump "{{target}}" --dry-run; \
    fi

release-notes:
    python3 ./scripts/release.py check

release-tag:
    python3 ./scripts/release.py tag

release-tag-sign:
    python3 ./scripts/release.py tag --sign

release *args:
    python3 ./scripts/release.py tag {{args}}

release-dry *args:
    python3 ./scripts/release.py tag --dry-run {{args}}

release-sign *args:
    python3 ./scripts/release.py tag --sign {{args}}

verify-full:
    python3 ./scripts/confidence.py full

release-check:
    python3 ./scripts/release.py check
    python3 ./scripts/confidence.py full
    cargo publish --dry-run --locked
