#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

pattern='\.env\("HOME"|\.env\("XDG_CONFIG_HOME"|\.env\("XDG_CACHE_HOME"|\.env\("XDG_STATE_HOME"'

if rg -n "$pattern" tests/contracts >/dev/null; then
  echo "contract tests must use tests/contracts/test_env.rs for isolated roots"
  rg -n "$pattern" tests/contracts
  exit 1
fi
