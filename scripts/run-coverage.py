#!/usr/bin/env python3
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

from coverage_workspace import clear_workspace_tmp, coverage_lock, prepare_workspace


def main() -> None:
    repo_root = Path(__file__).resolve().parent.parent
    with coverage_lock():
        _, env = prepare_workspace("osp-cov-manual-")
        subprocess.run(
            ["cargo", "llvm-cov", *sys.argv[1:]],
            cwd=repo_root,
            check=True,
            env=env,
        )
        clear_workspace_tmp()


if __name__ == "__main__":
    main()
