set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

fmt:
    python3 ./scripts/confidence.py --check fmt

fmt-fix:
    python3 ./scripts/confidence.py --check fmt-fix

clippy:
    python3 ./scripts/confidence.py --check clippy

test:
    python3 ./scripts/confidence.py --check test

audit:
    python3 ./scripts/confidence.py --check audit

build:
    python3 ./scripts/confidence.py --check build

install-tools lane='full':
    python3 ./scripts/confidence.py --install-tools {{lane}}

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
    python3 ./scripts/confidence.py --check coverage-summary

cov-gate:
    python3 ./scripts/confidence.py --check coverage

cov-gate-fast:
    python3 ./scripts/confidence.py --check coverage-fast

cov-baseline:
    python3 ./scripts/confidence.py --check coverage-baseline

startup-gate:
    python3 ./scripts/confidence.py --check startup-budget

startup-baseline:
    python3 ./scripts/confidence.py --check startup-baseline

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
    python3 ./scripts/confidence.py --check publish-dry-run
