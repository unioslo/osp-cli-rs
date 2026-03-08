#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

pattern='\.env\("HOME"|\.env\("XDG_CACHE_HOME"|\.env\("XDG_STATE_HOME"'

if rg -n "$pattern" crates/osp-cli/tests/contracts/*.rs >/dev/null; then
  echo "contract tests must use crates/osp-cli/tests/contracts/test_env.rs for isolated roots"
  rg -n "$pattern" crates/osp-cli/tests/contracts/*.rs
  exit 1
fi
