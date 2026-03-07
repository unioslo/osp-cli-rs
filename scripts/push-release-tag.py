#!/usr/bin/env python3
from __future__ import annotations

import argparse
import subprocess
import sys
import tomllib
from pathlib import Path


def fail(message: str) -> None:
    print(message, file=sys.stderr)
    raise SystemExit(1)


def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def workspace_version(root: Path) -> str:
    cargo = tomllib.loads((root / "Cargo.toml").read_text())
    return cargo["workspace"]["package"]["version"]


def tag_exists(root: Path, tag: str) -> bool:
    local = subprocess.run(
        ["git", "rev-parse", "--verify", "--quiet", f"refs/tags/{tag}"],
        cwd=root,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    if local.returncode == 0:
        return True

    remote = subprocess.run(
        ["git", "ls-remote", "--exit-code", "--tags", "origin", f"refs/tags/{tag}"],
        cwd=root,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    return remote.returncode == 0


def ensure_readiness(root: Path, tag: str) -> None:
    subprocess.run(
        ["python3", "./scripts/check-release-readiness.py", tag],
        cwd=root,
        check=True,
    )


def create_and_push_tag(root: Path, tag: str, signed: bool, dry_run: bool) -> None:
    tag_args = ["git", "tag", "-a", tag, "-m", f"Release {tag}"]
    if signed:
        tag_args = ["git", "tag", "-s", tag, "-m", f"Release {tag}"]

    push_args = ["git", "push", "origin", tag]

    if dry_run:
        print("would run:", " ".join(tag_args))
        print("would run:", " ".join(push_args))
        return

    subprocess.run(tag_args, cwd=root, check=True)
    subprocess.run(push_args, cwd=root, check=True)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Validate release readiness, create a tag, and push it safely."
    )
    parser.add_argument(
        "tag",
        nargs="?",
        help="Release tag to create, for example v0.1.0. Defaults to the workspace version.",
    )
    parser.add_argument(
        "--sign",
        action="store_true",
        help="Create a signed tag with git tag -s instead of an annotated tag.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print what would happen without creating or pushing the tag.",
    )
    args = parser.parse_args()

    root = repo_root()
    tag = args.tag or f"v{workspace_version(root)}"
    if not tag.startswith("v"):
        fail(f"release tag must start with 'v': {tag}")

    ensure_readiness(root, tag)

    if tag_exists(root, tag):
        fail(f"refusing to create or push existing tag: {tag}")

    create_and_push_tag(root, tag, signed=args.sign, dry_run=args.dry_run)
    if args.dry_run:
        print(f"Release tag dry run OK: {tag}")
    else:
        print(f"Release tag pushed: {tag}")


if __name__ == "__main__":
    main()
