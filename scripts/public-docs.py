#!/usr/bin/env python3
"""Public Rustdoc contract checks for osp-cli-rust.

This script enforces one narrow rule: exported Rust functions should carry
Rustdoc. It exists separately from `confidence.py` because the policy has two
distinct operating modes:

- `--staged` for fast pre-commit checks against the staged blob
- repo-wide mode for local confidence lanes and CI

That split is intentional. The hook should judge what is actually being
committed, not whatever happens to be in the working tree at the moment. The
repo-wide mode exists so the same contract can be enforced outside hooks.

This script deliberately uses lightweight syntax heuristics instead of a full
Rust parser. That keeps it fast and easy to run in hooks. Do not casually expand
it into a general documentation linter. If the team needs broader docs policy,
that should be a separate tool with different tradeoffs.

Warnings for future edits:

- preserve the staged-blob behavior; reading from the working tree weakens the
  hook contract
- keep test modules out of scope so internal helper churn does not become docs
  noise
- prefer a small number of obvious false negatives over complicated parsing that
  makes the hook slow or fragile
"""

from __future__ import annotations

import argparse
import pathlib
import re
import subprocess
import sys
from dataclasses import dataclass


PUB_FN_RE = re.compile(
    r"^\s*pub\s+(?:const\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\b"
)
TRAIT_FN_RE = re.compile(
    r"^\s*(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\b"
)
PUB_TRAIT_RE = re.compile(r"^\s*pub\s+trait\s+([A-Za-z_][A-Za-z0-9_]*)\b")
ATTR_RE = re.compile(r"^\s*#\s*\[")
DOC_RE = re.compile(r"^\s*(///|//!\s*|#\s*\[\s*doc\s*=)")


@dataclass(frozen=True)
class AddedLine:
    """One added staged line as reported by unified diff output."""

    path: pathlib.Path
    line_no: int


def repo_root() -> pathlib.Path:
    """Anchor git operations to the repository that owns this script."""

    return pathlib.Path(
        subprocess.run(
            ["git", "rev-parse", "--show-toplevel"],
            capture_output=True,
            text=True,
            check=True,
        )
        .stdout.strip()
    )


def is_internal_test_module(path: pathlib.Path) -> bool:
    """Keep internal test-only modules outside the public docs contract."""

    posix_path = path.as_posix()
    return posix_path.endswith("/tests.rs") or "/tests/" in posix_path


def staged_added_lines(root: pathlib.Path) -> list[AddedLine]:
    """Return added staged lines under `src/` for fast pre-commit enforcement."""

    cmd = [
        "git",
        "diff",
        "--cached",
        "--unified=0",
        "--no-color",
        "--",
        "src/**/*.rs",
        "src/*.rs",
    ]
    result = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        check=True,
        cwd=root,
    )
    lines = result.stdout.splitlines()

    out: list[AddedLine] = []
    current_path: pathlib.Path | None = None
    new_line_no: int | None = None

    for raw in lines:
        if raw.startswith("+++ b/"):
            current_path = pathlib.Path(raw[6:])
            new_line_no = None
            continue
        if raw.startswith("@@"):
            match = re.search(r"\+(\d+)(?:,(\d+))?", raw)
            if not match:
                new_line_no = None
                continue
            new_line_no = int(match.group(1))
            continue
        if current_path is None or new_line_no is None:
            continue
        if raw.startswith("+") and not raw.startswith("+++"):
            out.append(AddedLine(current_path, new_line_no))
            new_line_no += 1
        elif raw.startswith("-") and not raw.startswith("---"):
            continue
        else:
            new_line_no += 1

    return out


def staged_file_lines(root: pathlib.Path, path: pathlib.Path) -> list[str] | None:
    """Read the staged blob for a path instead of the working tree copy."""

    result = subprocess.run(
        ["git", "show", f":{path.as_posix()}"],
        capture_output=True,
        text=True,
        check=False,
        cwd=root,
    )
    if result.returncode != 0:
        return None
    return result.stdout.splitlines()


def has_rustdoc(lines: list[str], line_no: int) -> bool:
    """Check whether a function-like line is preceded by Rustdoc.

    The search intentionally tolerates attributes and blank lines because those
    are common around exported functions and trait methods.
    """

    index = line_no - 2
    saw_doc = False

    while index >= 0:
        raw = lines[index]
        stripped = raw.strip()
        if not stripped:
            if saw_doc:
                return True
            index -= 1
            continue
        if DOC_RE.match(raw):
            saw_doc = True
            index -= 1
            continue
        if ATTR_RE.match(raw):
            index -= 1
            continue
        return saw_doc

    return saw_doc


def is_pub_trait_method(lines: list[str], line_no: int) -> bool:
    """Heuristically detect methods inside a public trait block.

    A lightweight brace-depth scan is good enough here and keeps the hook much
    cheaper than introducing a full parser.
    """

    brace_depth = 0
    trait_depths: list[int] = []

    for raw in lines[: line_no - 1]:
        stripped = raw.strip()
        if PUB_TRAIT_RE.match(raw) and "{" in raw:
            trait_depths.append(brace_depth)
        opens = raw.count("{")
        closes = raw.count("}")
        brace_depth += opens - closes
        while trait_depths and brace_depth <= trait_depths[-1]:
            trait_depths.pop()
        if PUB_TRAIT_RE.match(raw) and "{" not in raw:
            # This intentionally favors the common code shape over perfect Rust
            # parsing because hook speed and simplicity matter more here.
            continue

    return bool(trait_depths)


def missing_docs_in_lines(
    path: pathlib.Path,
    lines: list[str],
    line_numbers: set[int] | None = None,
) -> list[str]:
    """Return missing-doc failures for exported functions in the given lines."""

    failures: list[str] = []
    limit_to = line_numbers
    for line_no, line in enumerate(lines, start=1):
        if limit_to is not None and line_no not in limit_to:
            continue
        match = PUB_FN_RE.match(line)
        if match:
            name = match.group(1)
        elif is_pub_trait_method(lines, line_no):
            trait_match = TRAIT_FN_RE.match(line)
            if not trait_match:
                continue
            name = trait_match.group(1)
        else:
            continue
        if has_rustdoc(lines, line_no):
            continue
        failures.append(
            f"{path}:{line_no}: public function `{name}` is missing a Rustdoc comment"
        )
    return failures


def check_staged_public_functions(root: pathlib.Path) -> list[str]:
    """Evaluate the public docs contract against staged additions only."""

    failures: list[str] = []
    by_file: dict[pathlib.Path, set[int]] = {}
    for added in staged_added_lines(root):
        if is_internal_test_module(added.path):
            continue
        by_file.setdefault(added.path, set()).add(added.line_no)

    for path, line_numbers in sorted(by_file.items()):
        lines = staged_file_lines(root, path)
        if lines is None:
            continue
        failures.extend(missing_docs_in_lines(path, lines, line_numbers))

    return failures


def rust_source_files(root: pathlib.Path) -> list[pathlib.Path]:
    """Return production Rust source files that participate in this contract."""

    return sorted(
        path.relative_to(root)
        for path in root.joinpath("src").rglob("*.rs")
        if not is_internal_test_module(path.relative_to(root))
    )


def check_workspace_public_functions(root: pathlib.Path) -> list[str]:
    """Evaluate the public docs contract across the whole repository tree."""

    failures: list[str] = []
    for path in rust_source_files(root):
        lines = root.joinpath(path).read_text().splitlines()
        failures.extend(missing_docs_in_lines(path, lines))
    return failures


def main() -> int:
    """Parse CLI mode and run the public docs contract check."""

    parser = argparse.ArgumentParser(
        description="Check public Rust functions for missing Rustdoc comments."
    )
    parser.add_argument(
        "--staged",
        action="store_true",
        help="Check only staged added lines, intended for pre-commit hooks.",
    )
    args = parser.parse_args()

    root = repo_root()
    if args.staged:
        failures = check_staged_public_functions(root)
    else:
        failures = check_workspace_public_functions(root)
    if not failures:
        return 0

    if args.staged:
        print(
            "ERROR: staged public Rust functions must have Rustdoc comments.",
            file=sys.stderr,
        )
    else:
        print(
            "ERROR: public Rust functions must have Rustdoc comments.",
            file=sys.stderr,
        )
    print(file=sys.stderr)
    for failure in failures:
        print(f"  {failure}", file=sys.stderr)
    print(file=sys.stderr)
    print(
        "Add a `///` Rustdoc comment immediately above the exported function.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
