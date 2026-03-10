#!/usr/bin/env python3
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
    path: pathlib.Path
    line_no: int


def staged_added_lines() -> list[AddedLine]:
    repo_root = (
        subprocess.run(
            ["git", "rev-parse", "--show-toplevel"],
            capture_output=True,
            text=True,
            check=True,
        )
        .stdout.strip()
    )
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
        cwd=repo_root,
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


def has_rustdoc(lines: list[str], line_no: int) -> bool:
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
            # Multi-line trait declarations are rare here; keep the hook strict
            # for the common case instead of trying to parse Rust fully.
            continue

    return bool(trait_depths)


def check_staged_public_functions() -> list[str]:
    failures: list[str] = []

    by_file: dict[pathlib.Path, set[int]] = {}
    for added in staged_added_lines():
        by_file.setdefault(added.path, set()).add(added.line_no)

    for path, line_numbers in sorted(by_file.items()):
        if not path.exists():
            continue
        lines = path.read_text().splitlines()
        for line_no in sorted(line_numbers):
            if line_no < 1 or line_no > len(lines):
                continue
            line = lines[line_no - 1]
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


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Block staged public Rust functions without Rustdoc comments."
    )
    parser.parse_args()

    failures = check_staged_public_functions()
    if not failures:
        return 0

    print("ERROR: staged public Rust functions must have Rustdoc comments.", file=sys.stderr)
    print(file=sys.stderr)
    for failure in failures:
        print(f"  {failure}", file=sys.stderr)
    print(file=sys.stderr)
    print("Add a `///` Rustdoc comment immediately above the exported function.", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
