#!/usr/bin/env python3
from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tomllib
from pathlib import Path


def fail(message: str) -> None:
    print(message, file=sys.stderr)
    raise SystemExit(1)


def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def load_workspace_version(cargo_toml: Path) -> str:
    payload = tomllib.loads(cargo_toml.read_text())
    return payload["workspace"]["package"]["version"]


def parse_version(version: str) -> tuple[int, int, int]:
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)", version)
    if not match:
        fail(f"unsupported workspace version format: {version}")
    return tuple(int(part) for part in match.groups())


def bump_version(version: str, kind: str) -> str:
    major, minor, patch = parse_version(version)
    if kind == "patch":
        patch += 1
    elif kind == "minor":
        minor += 1
        patch = 0
    elif kind == "major":
        major += 1
        minor = 0
        patch = 0
    else:
        fail(f"unsupported bump kind: {kind}")
    return f"{major}.{minor}.{patch}"


def validate_version(version: str) -> str:
    parse_version(version)
    return version


def resolve_target_version(current: str, target: str) -> str:
    if target in {"patch", "minor", "major"}:
        return bump_version(current, target)

    next_version = validate_version(target)
    if parse_version(next_version) <= parse_version(current):
        fail(
            f"explicit version must be greater than current version: "
            f"{next_version} <= {current}"
        )
    return next_version


def replace_workspace_version(cargo_toml: Path, old: str, new: str) -> None:
    text = cargo_toml.read_text()
    pattern = re.compile(
        r"(?ms)^(\[workspace\.package\]\n(?:.*\n)*?version = \")([^\"]+)(\")"
    )
    replaced, count = pattern.subn(rf"\g<1>{new}\g<3>", text, count=1)
    if count != 1:
        fail("failed to update workspace.package.version in Cargo.toml")
    cargo_toml.write_text(replaced)


def workspace_package_names(root: Path) -> list[str]:
    cargo_toml = tomllib.loads((root / "Cargo.toml").read_text())
    members = cargo_toml["workspace"]["members"]
    names: list[str] = []
    for member in members:
        manifest = tomllib.loads((root / member / "Cargo.toml").read_text())
        names.append(manifest["package"]["name"])
    return names


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


def replace_lock_versions(lockfile: Path, package_names: list[str], new_version: str) -> None:
    lines = lockfile.read_text().splitlines()
    out: list[str] = []
    in_package = False
    current_name: str | None = None
    for line in lines:
        if line == "[[package]]":
            in_package = True
            current_name = None
            out.append(line)
            continue
        if in_package and line.startswith("name = "):
            current_name = line.split('"')[1]
            out.append(line)
            continue
        if in_package and line.startswith("version = ") and current_name in package_names:
            out.append(f'version = "{new_version}"')
            continue
        out.append(line)
    lockfile.write_text("\n".join(out) + "\n")


def release_notes_template(version: str, message: str | None) -> str:
    summary = (
        f"- {message}\n"
        if message
        else "TODO: summarize the release in 2-4 bullets.\n"
    )
    highlight = f"- {message}\n" if message else "- TODO\n"
    return f"""# Release v{version}

## Summary

{summary}

## Highlights

{highlight}

## Verification

- TODO: note the release verification commands or checks that mattered.
"""


def changelog_template() -> str:
    return """# Changelog

All notable changes to this project will be documented in this file.

The current release flow requires one versioned changelog section and one
matching `docs/releases/vX.Y.Z.md` file before a tag can be published.
"""


def changelog_entry_template(version: str, message: str | None) -> str:
    bullet = message or "TODO: summarize the release in 1-3 bullets."
    return f"""

## [{version}] - YYYY-MM-DD

- {bullet}
"""


def ensure_release_notes(
    root: Path, version: str, dry_run: bool, message: str | None
) -> Path:
    release_notes = root / "docs" / "releases" / f"v{version}.md"
    if release_notes.exists():
        return release_notes
    if not dry_run:
        release_notes.parent.mkdir(parents=True, exist_ok=True)
        release_notes.write_text(release_notes_template(version, message))
    return release_notes


def ensure_changelog_entry(
    root: Path, version: str, dry_run: bool, message: str | None
) -> Path:
    changelog = root / "CHANGELOG.md"
    if not changelog.exists():
        if dry_run:
            return changelog
        changelog.write_text(changelog_template())

    body = changelog.read_text()
    section_pattern = re.compile(
        rf"(?m)^## \[(?:v)?{re.escape(version)}\](?: - .+)?$"
    )
    if section_pattern.search(body):
        return changelog

    if dry_run:
        return changelog

    updated = body.rstrip() + changelog_entry_template(version, message) + "\n"
    changelog.write_text(updated)
    return changelog


def run_lock_refresh(root: Path, dry_run: bool) -> None:
    if dry_run:
        return
    subprocess.run(
        ["cargo", "metadata", "--format-version", "1"],
        cwd=root,
        check=True,
        stdout=subprocess.DEVNULL,
    )


def main() -> None:
    parser = argparse.ArgumentParser(description="Bump the workspace version.")
    parser.add_argument(
        "target",
        nargs="?",
        default="patch",
        help="One of patch/minor/major, or an explicit semantic version like 1.4.5.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print the next version without editing files.",
    )
    parser.add_argument(
        "-m",
        "--message",
        help="Seed the new changelog and release notes with a short summary.",
    )
    args = parser.parse_args()

    root = repo_root()
    cargo_toml = root / "Cargo.toml"
    lockfile = root / "Cargo.lock"

    current = load_workspace_version(cargo_toml)
    next_version = resolve_target_version(current, args.target)
    release_notes = ensure_release_notes(
        root, next_version, args.dry_run, args.message
    )
    changelog = ensure_changelog_entry(
        root, next_version, args.dry_run, args.message
    )
    next_tag = f"v{next_version}"

    if tag_exists(root, next_tag):
        fail(f"refusing to bump to {next_version}: tag already exists: {next_tag}")

    if args.dry_run:
        print(f"current={current}")
        print(f"next={next_version}")
        print(f"tag={next_tag}")
        print(f"release_notes={release_notes.relative_to(root)}")
        print(f"changelog={changelog.relative_to(root)}")
        return

    replace_workspace_version(cargo_toml, current, next_version)
    replace_lock_versions(lockfile, workspace_package_names(root), next_version)
    run_lock_refresh(root, dry_run=False)

    print(f"bumped version: {current} -> {next_version}")
    print(f"tag: {next_tag}")
    print(f"release notes: {release_notes.relative_to(root)}")
    print(f"changelog: {changelog.relative_to(root)}")


if __name__ == "__main__":
    main()
