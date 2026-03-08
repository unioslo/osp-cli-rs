#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

./scripts/check-contract-env.sh

cargo fmt --all --check

cargo clippy \
  -p osp-cli \
  -p osp-completion \
  -p osp-config \
  -p osp-dsl \
  -p osp-repl \
  -p osp-ui \
  --all-targets \
  -- \
  -D clippy::collapsible_else_if \
  -D clippy::collapsible_if \
  -D clippy::derivable_impls \
  -D clippy::get_first \
  -D clippy::io_other_error \
  -D clippy::lines_filter_map_ok \
  -D clippy::manual_pattern_char_comparison \
  -D clippy::match_like_matches_macro \
  -D clippy::needless_as_bytes \
  -D clippy::needless_borrow \
  -D clippy::question_mark \
  -D clippy::redundant_closure \
  -D clippy::unnecessary_lazy_evaluations
