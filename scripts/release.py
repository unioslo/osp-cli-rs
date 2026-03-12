#!/usr/bin/env python3
"""Release workflow helpers for osp-cli-rust.

This file owns one narrow operational domain: taking the repository from "the
next release is being prepared" to "the release is ready to tag and publish".
It deliberately concentrates that workflow behind three explicit subcommands:

- `bump` prepares the next version locally and scaffolds release artifacts
- `check` validates that the current repository state is releasable
- `tag` re-runs readiness checks, creates the git tag, and pushes it safely

The project already has a lot of testing and policy machinery elsewhere. This
script is not meant to be another generic task runner. It should stay focused on
release invariants that are easy to audit in review:

- the package version is the source of truth for the release version
- release notes and changelog state must exist and must not contain placeholders
- tagging is stricter than bumping because it is the step closest to publication
- local convenience must never quietly weaken release safety checks

Why keep this in one script:

- release helpers belong to one workflow domain, unlike `confidence.py`,
  `coverage.py`, and `public-docs.py`, which intentionally own separate
  confidence-policy concerns
- `just`, CI, and human operators should all see the same command surface
- the release flow should read as a small policy document, not as a set of
  drifting helper files

Warnings for future edits:

- do not make `bump` depend on remote connectivity; preparing the next version
  should remain possible offline
- do not make `tag` permissive on remote-check failures; inability to verify the
  remote is a release-blocking condition, not a soft warning
- do not broaden placeholder exceptions casually; false positives are cheaper
  than shipping unfinished release notes
- do not grow this into a release framework with shared mutable state or hidden
  sequencing; explicit subcommands and obvious side effects are part of the
  safety model

If this file gets substantially more clever, it will get less trustworthy.
Prefer explicit workflow steps, obvious error messages, and helpers that exist
to explain policy boundaries rather than to hide them.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import Any


def fail(message: str) -> None:
    """Exit with a user-facing error message."""

    print(message, file=sys.stderr)
    raise SystemExit(1)


def repo_root() -> Path:
    """Anchor release operations to the repository that owns this script."""

    return Path(__file__).resolve().parent.parent


def load_cargo_package(root: Path) -> dict[str, Any]:
    """Load the root Cargo package metadata used by release policy."""

    payload = tomllib.loads((root / "Cargo.toml").read_text())
    return payload["package"]


def package_name(root: Path) -> str:
    """Return the root package name for lockfile updates."""

    return load_cargo_package(root)["name"]


def package_version(root: Path) -> str:
    """Return the current root package version."""

    return load_cargo_package(root)["version"]


def parse_version(version: str) -> tuple[int, int, int]:
    """Parse strict `major.minor.patch` versions used by the release flow."""

    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)", version)
    if not match:
        fail(f"unsupported package version format: {version}")
    return tuple(int(part) for part in match.groups())


def normalize_version(value: str) -> str:
    """Accept either `1.2.3` or `v1.2.3` at the CLI boundary."""

    return value[1:] if value.startswith("v") else value


def validate_version(value: str) -> str:
    """Validate and normalize a semantic version string."""

    version = normalize_version(value)
    parse_version(version)
    return version


def bump_version(version: str, kind: str) -> str:
    """Return the next semantic version for a named bump kind."""

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


def resolve_target_version(current: str, target: str) -> str:
    """Resolve a bump target into a concrete next version."""

    if target in {"patch", "minor", "major"}:
        return bump_version(current, target)

    next_version = validate_version(target)
    if parse_version(next_version) <= parse_version(current):
        fail(
            f"explicit version must be greater than current version: "
            f"{next_version} <= {current}"
        )
    return next_version


def release_tag(version: str) -> str:
    """Format the canonical git tag for a version."""

    return f"v{version}"


def release_notes_path(root: Path, version: str) -> Path:
    """Return the checked-in release notes path for a version."""

    return root / "docs" / "releases" / f"v{version}.md"


def changelog_path(root: Path) -> Path:
    """Return the repository changelog path."""

    return root / "CHANGELOG.md"


def local_tag_exists(root: Path, tag: str) -> bool:
    """Check for an existing local tag without consulting remotes."""

    result = subprocess.run(
        ["git", "rev-parse", "--verify", "--quiet", f"refs/tags/{tag}"],
        cwd=root,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    return result.returncode == 0


def clean_worktree_entries(root: Path) -> list[str]:
    """Return tracked and untracked worktree changes relevant to release safety."""

    result = subprocess.run(
        ["git", "status", "--porcelain"],
        cwd=root,
        text=True,
        capture_output=True,
        check=True,
    )
    return [line for line in result.stdout.splitlines() if line.strip()]


def ensure_clean_worktree(root: Path, *, purpose: str) -> None:
    """Refuse risky release operations when the worktree is not clean.

    Release work is easier to reason about when the repository state is
    intentional and reproducible. A dirty tree makes it too easy to tag or bump
    in the middle of unrelated edits, then lose track of what was actually
    verified.
    """

    entries = clean_worktree_entries(root)
    if not entries:
        return

    preview = "\n".join(f"  {line}" for line in entries[:10])
    suffix = "\n  ..." if len(entries) > 10 else ""
    fail(
        f"refusing to {purpose} with a dirty worktree.\n"
        "Commit, stash, or clean these changes first:\n"
        f"{preview}{suffix}"
    )


def ensure_origin_remote(root: Path) -> None:
    """Require an `origin` remote before attempting release-tag publication.

    The release flow assumes `origin` is the publication remote. If that stops
    being true, the workflow should be redesigned deliberately rather than
    inferred from whatever remotes happen to exist locally.
    """

    result = subprocess.run(
        ["git", "remote", "get-url", "origin"],
        cwd=root,
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode == 0:
        return

    detail = result.stderr.strip() or "remote 'origin' is not configured"
    fail(f"cannot verify remote tag state on origin: {detail}")


def ensure_remote_tag_absent(root: Path, tag: str) -> None:
    """Require a successful remote tag lookup before concluding a tag is free.

    Release tagging is the strict step in the workflow. Network, auth, or remote
    configuration failures are not treated as "tag absent" because that would
    silently convert a safety check into best-effort advice.
    """

    ensure_origin_remote(root)

    env = dict(os.environ)
    env["GIT_TERMINAL_PROMPT"] = "0"
    env.setdefault("GIT_SSH_COMMAND", "ssh -o BatchMode=yes")
    result = subprocess.run(
        ["git", "ls-remote", "--exit-code", "--tags", "origin", f"refs/tags/{tag}"],
        cwd=root,
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode == 0:
        fail(f"refusing to create or push existing tag: {tag}")
    if result.returncode == 2:
        return

    detail = result.stderr.strip() or result.stdout.strip()
    if not detail:
        detail = f"`git ls-remote` exited with status {result.returncode}"
    fail(f"cannot verify remote tag state for {tag} on origin: {detail}")


def replace_root_version(cargo_toml: Path, new_version: str) -> None:
    """Rewrite the root `package.version` entry in Cargo.toml.

    This intentionally relies on the repository's canonical root manifest layout
    instead of introducing a TOML writer dependency just for one field update.
    That is a tradeoff, not a general endorsement of regex-editing TOML. If the
    manifest structure becomes less regular, revisit this helper rather than
    broadening the regex further.
    """

    text = cargo_toml.read_text()
    pattern = re.compile(r'(?ms)^(\[package\]\n(?:.*\n)*?version = ")([^"]+)(")')
    replaced, count = pattern.subn(rf"\g<1>{new_version}\g<3>", text, count=1)
    if count != 1:
        fail("failed to update package.version in Cargo.toml")
    cargo_toml.write_text(replaced)


def replace_lock_versions(lockfile: Path, package_names: list[str], new_version: str) -> None:
    """Rewrite root package versions in Cargo.lock after a version bump.

    Matching by package name is acceptable here because this repository has a
    small, explicit root-package release surface. This helper is intentionally
    narrow and repo-shaped; if the release surface becomes more complex, this
    should be tightened rather than silently trusted.
    """

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


def run_lock_refresh(root: Path, *, dry_run: bool) -> None:
    """Refresh Cargo lock/metadata after a real bump.

    This keeps Cargo's derived state aligned with the rewritten manifest and
    lockfile without asking contributors to remember a second command. The
    release path should prefer one obvious workflow over "remember to run these
    three follow-up repair commands".
    """

    if dry_run:
        return
    subprocess.run(
        ["cargo", "metadata", "--format-version", "1"],
        cwd=root,
        check=True,
        stdout=subprocess.DEVNULL,
    )


def release_notes_template(version: str, message: str | None) -> str:
    """Return the starter release-notes document for a new version."""

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
    """Return the initial changelog scaffold when none exists yet."""

    return """# Changelog

All notable changes to this project will be documented in this file.

The current release flow requires one versioned changelog section and one
matching `docs/releases/vX.Y.Z.md` file before a tag can be published.
"""


def changelog_entry_template(version: str, message: str | None) -> str:
    """Return the starter changelog section for a new version."""

    bullet = message or "TODO: summarize the release in 1-3 bullets."
    return f"""

## [{version}] - YYYY-MM-DD

- {bullet}
"""


def ensure_release_notes(
    root: Path, version: str, *, dry_run: bool, message: str | None
) -> Path:
    """Create release notes when bumping to a fresh version."""

    notes = release_notes_path(root, version)
    if notes.exists():
        return notes
    if not dry_run:
        notes.parent.mkdir(parents=True, exist_ok=True)
        notes.write_text(release_notes_template(version, message))
    return notes


def ensure_changelog_entry(
    root: Path, version: str, *, dry_run: bool, message: str | None
) -> Path:
    """Create the changelog file or version section when bumping."""

    changelog = changelog_path(root)
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


def validate_release_notes(path: Path) -> None:
    """Reject missing, empty, or placeholder-filled release notes.

    Placeholder detection is deliberately conservative. A false positive is
    preferable to publishing a release with unfinished notes because the cost of
    re-running a local check is much lower than the cost of shipping sloppy
    release metadata.
    """

    if not path.exists():
        fail(f"missing release notes file: {path}")
    body = path.read_text().strip()
    if not body:
        fail(f"release notes file is empty: {path}")
    if "TODO" in body:
        fail(f"release notes still contain TODO placeholders: {path}")


def extract_changelog_section(body: str, version: str) -> str | None:
    """Return one version section from the changelog body, if present."""

    pattern = re.compile(
        rf"(?ms)^## \[(?:v)?{re.escape(version)}\](?: - .+)?\n(.*?)(?=^## \[|\Z)"
    )
    match = pattern.search(body)
    if not match:
        return None
    return match.group(1).strip()


def validate_changelog(path: Path, version: str) -> None:
    """Reject missing, empty, or placeholder-filled changelog state.

    Placeholder detection is deliberately conservative for the same reason as
    release notes validation: this is release policy, not free-form prose lint,
    and the workflow should bias toward blocking ambiguous release state.
    """

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


def check_release_readiness(root: Path, version_or_tag: str | None) -> str:
    """Validate that the current package version is ready to release.

    The CLI may pass either a version or a tag. This function always normalizes
    to a plain semantic version and then requires that it match the root package
    version exactly. That strictness is intentional: release metadata drift is a
    more important failure to catch than user convenience at this layer.
    """

    expected = package_version(root)
    requested = validate_version(version_or_tag or expected)
    if requested != expected:
        fail(
            f"requested release version {requested} does not match package version {expected}"
        )

    notes = release_notes_path(root, requested)
    validate_release_notes(notes)
    changelog = changelog_path(root)
    validate_changelog(changelog, requested)
    return requested


def create_and_push_tag(root: Path, tag: str, *, signed: bool, dry_run: bool) -> None:
    """Create the git tag locally and push it, or print the plan in dry-run mode.

    The creation and push steps are kept adjacent because splitting them across
    separate helpers makes it easier for future edits to accidentally skip one of
    the safety checks in between.
    """

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


def command_bump(args: argparse.Namespace) -> None:
    """Handle version bump preparation and release-scaffold updates.

    The sequence here is intentionally shaped to feel transactional:

    - reject obviously bad targets and tag collisions first
    - require a clean tree for real writes
    - rewrite version-bearing state
    - then scaffold docs/changelog for the new version

    The docs scaffolding comes after version writes on purpose so a failed bump
    is less likely to leave behind release artifacts for a version the repository
    never actually adopted.
    """

    root = repo_root()
    cargo_toml = root / "Cargo.toml"
    root_lockfile = root / "Cargo.lock"

    current = package_version(root)
    next_version = resolve_target_version(current, args.target)
    notes = release_notes_path(root, next_version)
    changelog = changelog_path(root)
    next_tag = release_tag(next_version)

    if local_tag_exists(root, next_tag):
        fail(f"refusing to bump to {next_version}: tag already exists: {next_tag}")
    if not args.dry_run:
        ensure_clean_worktree(root, purpose="bump the release version")

    if args.dry_run:
        print(f"current={current}")
        print(f"next={next_version}")
        print(f"tag={next_tag}")
        print(f"release_notes={notes.relative_to(root)}")
        print(f"changelog={changelog.relative_to(root)}")
        return

    # Version-bearing state changes first so later scaffolding reflects a repo
    # that has actually moved to the new version.
    replace_root_version(cargo_toml, next_version)
    replace_lock_versions(root_lockfile, [package_name(root)], next_version)
    run_lock_refresh(root, dry_run=False)
    notes = ensure_release_notes(root, next_version, dry_run=False, message=args.message)
    changelog = ensure_changelog_entry(
        root, next_version, dry_run=False, message=args.message
    )

    print(f"bumped version: {current} -> {next_version}")
    print(f"tag: {next_tag}")
    print(f"release notes: {notes.relative_to(root)}")
    print(f"changelog: {changelog.relative_to(root)}")


def command_check(args: argparse.Namespace) -> None:
    """Handle the release-readiness contract subcommand.

    This is the policy-only step: it should stay free of side effects so humans,
    CI, and the `tag` command can all trust it as the same contract.
    """

    root = repo_root()
    version = check_release_readiness(root, args.version)
    notes = release_notes_path(root, version)
    changelog = changelog_path(root)
    print(
        "Release readiness OK for "
        f"v{version}: {notes.relative_to(root)}, {changelog.relative_to(root)}"
    )


def command_tag(args: argparse.Namespace) -> None:
    """Handle readiness verification, tag creation, and tag push.

    This is the strictest step in the release flow. It should stay boring and
    conservative:

    - require a clean tree
    - require readiness to pass again
    - require both local and remote tag safety checks to succeed
    - only then create and push the tag

    If a future refactor weakens any of those checks for convenience, it is
    probably making the release path worse.
    """

    root = repo_root()
    version = package_version(root)
    tag = args.tag or release_tag(version)
    if not tag.startswith("v"):
        fail(f"release tag must start with 'v': {tag}")

    ensure_clean_worktree(root, purpose="create and push a release tag")
    check_release_readiness(root, tag)

    if local_tag_exists(root, tag):
        fail(f"refusing to create or push existing tag: {tag}")
    ensure_remote_tag_absent(root, tag)

    create_and_push_tag(root, tag, signed=args.sign, dry_run=args.dry_run)
    if args.dry_run:
        print(f"Release tag dry run OK: {tag}")
    else:
        print(f"Release tag pushed: {tag}")


def build_parser() -> argparse.ArgumentParser:
    """Build the release helper CLI with explicit subcommands."""

    parser = argparse.ArgumentParser(
        description="Release workflow helpers for osp-cli-rust."
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    bump_parser = subparsers.add_parser(
        "bump",
        help="Bump the root package version and scaffold release artifacts.",
    )
    bump_parser.add_argument(
        "target",
        nargs="?",
        default="patch",
        help="One of patch/minor/major, or an explicit semantic version like 1.4.5.",
    )
    bump_parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print the next version without editing files.",
    )
    bump_parser.add_argument(
        "-m",
        "--message",
        help="Seed the new changelog and release notes with a short summary.",
    )
    bump_parser.set_defaults(func=command_bump)

    check_parser = subparsers.add_parser(
        "check",
        help="Validate release readiness for the current package version.",
    )
    check_parser.add_argument(
        "version",
        nargs="?",
        help=(
            "Version or tag to validate, for example 0.1.0 or v0.1.0. "
            "Defaults to the root package version."
        ),
    )
    check_parser.set_defaults(func=command_check)

    tag_parser = subparsers.add_parser(
        "tag",
        help="Validate readiness, create a tag, and push it safely.",
    )
    tag_parser.add_argument(
        "tag",
        nargs="?",
        help=(
            "Release tag to create, for example v0.1.0. "
            "Defaults to the root package version."
        ),
    )
    tag_parser.add_argument(
        "--sign",
        action="store_true",
        help="Create a signed tag with git tag -s instead of an annotated tag.",
    )
    tag_parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print what would happen without creating or pushing the tag.",
    )
    tag_parser.set_defaults(func=command_tag)

    return parser


def main() -> None:
    """Parse CLI arguments and dispatch to the selected release subcommand."""

    parser = build_parser()
    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
