#!/usr/bin/env python3
"""Run one cargo command under a deterministic test runtime environment.

This is intentionally narrow. It is not a generic task runner and it should
not grow test selection logic. Its job is to strip ambient host/runtime state
that leaks into osp test execution while leaving toolchain/build variables
alone.
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
import tempfile
from pathlib import Path


def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def hermetic_env(home: Path) -> dict[str, str]:
    env = dict(os.environ)
    original_home = Path(os.environ.get("HOME", str(Path.home())))

    prefixes_to_remove = ("OSP_",)
    exact_to_remove = {
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "XDG_STATE_HOME",
        "NO_COLOR",
        "COLORTERM",
        "CLICOLOR",
        "CLICOLOR_FORCE",
        "LS_COLORS",
        "GREP_COLOR",
        "GREP_COLORS",
        "TERM",
    }

    for key in list(env):
        if key in exact_to_remove or any(key.startswith(prefix) for prefix in prefixes_to_remove):
            env.pop(key, None)

    env.update(
        {
            "HOME": str(home),
            "XDG_CONFIG_HOME": str(home / ".config"),
            "XDG_CACHE_HOME": str(home / ".cache"),
            "XDG_STATE_HOME": str(home / ".local" / "state"),
            "CARGO_HOME": env.get("CARGO_HOME", str(original_home / ".cargo")),
            "RUSTUP_HOME": env.get("RUSTUP_HOME", str(original_home / ".rustup")),
            "LANG": "C.UTF-8",
            "TERM": "dumb",
            "NO_COLOR": "1",
        }
    )
    return env


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Run a cargo command under a hermetic osp runtime env."
    )
    parser.add_argument(
        "command",
        nargs=argparse.REMAINDER,
        help="Command to run, for example: cargo test --test integration --locked",
    )
    args = parser.parse_args()

    command = args.command
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        parser.error("missing command")

    with tempfile.TemporaryDirectory(prefix="osp-cli-hermetic-home-") as home:
        env = hermetic_env(Path(home))
        completed = subprocess.run(command, cwd=repo_root(), env=env)
        return completed.returncode


if __name__ == "__main__":
    raise SystemExit(main())
