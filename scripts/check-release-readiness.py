#!/usr/bin/env python3
from __future__ import annotations

import argparse
import re
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


def normalize_version(value: str) -> str:
    return value[1:] if value.startswith("v") else value


def validate_version(value: str) -> str:
    version = normalize_version(value)
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        fail(f"expected a semantic version like 0.1.0 or v0.1.0, got: {value}")
    return version


def release_notes_path(root: Path, version: str) -> Path:
    return root / "docs" / "releases" / f"v{version}.md"


def changelog_path(root: Path) -> Path:
    return root / "CHANGELOG.md"


def validate_release_notes(path: Path) -> None:
    if not path.exists():
        fail(f"missing release notes file: {path}")
    body = path.read_text().strip()
    if not body:
        fail(f"release notes file is empty: {path}")
    if "TODO" in body:
        fail(f"release notes still contain TODO placeholders: {path}")


def extract_changelog_section(body: str, version: str) -> str | None:
    pattern = re.compile(
        rf"(?ms)^## \[(?:v)?{re.escape(version)}\](?: - .+)?\n(.*?)(?=^## \[|\Z)"
    )
    match = pattern.search(body)
    if not match:
        return None
    return match.group(1).strip()


def validate_changelog(path: Path, version: str) -> None:
    if not path.exists():
        fail(f"missing changelog file: {path}")
    body = path.read_text().strip()
    if not body:
        fail(f"changelog file is empty: {path}")

    section = extract_changelog_section(body, version)
    if section is None:
        fail(f"missing changelog section for v{version}: {path}")
    if "TODO" in section or "YYYY-MM-DD" in section:
        fail(f"changelog section for v{version} still has placeholders: {path}")


def main() -> None:
    parser = argparse.ArgumentParser(description="Validate release notes for a version.")
    parser.add_argument(
        "version",
        nargs="?",
        help="Version or tag to validate, for example 0.1.0 or v0.1.0. Defaults to workspace version.",
    )
    args = parser.parse_args()

    root = repo_root()
    expected = workspace_version(root)
    requested = validate_version(args.version or expected)
    if requested != expected:
        fail(
            f"release notes version {requested} does not match workspace version {expected}"
        )

    notes = release_notes_path(root, requested)
    validate_release_notes(notes)
    changelog = changelog_path(root)
    validate_changelog(changelog, requested)
    print(
        "Release readiness OK for "
        f"v{requested}: {notes.relative_to(root)}, {changelog.relative_to(root)}"
    )


if __name__ == "__main__":
    main()
