#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

pattern='\.env\("HOME"|\.env\("XDG_CONFIG_HOME"|\.env\("XDG_CACHE_HOME"|\.env\("XDG_STATE_HOME"'

status=0
matches="$(rg -n "$pattern" tests/contracts)" || status=$?
if ((status == 0)); then
  echo "contract tests must use tests/contracts/test_env.rs for isolated roots"
  printf '%s\n' "$matches"
  exit 1
fi
if ((status != 1)); then
  echo "contract environment search failed (rg exit $status)" >&2
  exit "$status"
fi
