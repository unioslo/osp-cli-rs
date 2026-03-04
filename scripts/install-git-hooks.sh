#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

git config core.hooksPath .githooks
git config commit.template .gitmessage

echo "Configured git hooks path: .githooks"
echo "Configured commit template: .gitmessage"
