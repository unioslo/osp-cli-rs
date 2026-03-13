#!/usr/bin/env python3
"""Check the curated runnable-doctest baseline for selected public symbols."""

from __future__ import annotations

import pathlib
import re
import subprocess
import sys


PUB_FN_RE = re.compile(
    r"^\s*pub\s+(?:const\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\b"
)
TRAIT_FN_RE = re.compile(r"^\s*(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\b")
PUB_TRAIT_RE = re.compile(r"^\s*pub\s+trait\s+([A-Za-z_][A-Za-z0-9_]*)\b")
DOC_RE = re.compile(r"^\s*(///|//!\s*|#\s*\[\s*doc\s*=)")
IMPL_RE = re.compile(r"^\s*impl\b")
IMPL_OWNER_RE = re.compile(
    r"^\s*impl(?:<[^{};]*>)?\s+([A-Za-z_][A-Za-z0-9_]*)\b"
)
IMPL_TRAIT_OWNER_RE = re.compile(
    r"^\s*impl(?:<[^{};]*>)?\s+.+\s+for\s+([A-Za-z_][A-Za-z0-9_]*)\b"
)

NON_RUST_FENCE_TOKENS = {
    "bash",
    "console",
    "ignore",
    "json",
    "markdown",
    "md",
    "no_run",
    "plaintext",
    "python",
    "sh",
    "text",
    "toml",
    "yaml",
    "compile_fail",
}


def repo_root() -> pathlib.Path:
    return pathlib.Path(
        subprocess.run(
            ["git", "rev-parse", "--show-toplevel"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
    )


def load_baseline(root: pathlib.Path) -> list[tuple[pathlib.Path, str]]:
    baseline = root / ".public-api-examples.txt"
    entries: list[tuple[pathlib.Path, str]] = []
    for raw in baseline.read_text().splitlines():
        stripped = raw.strip()
        if not stripped or stripped.startswith("#"):
            continue
        try:
            raw_path, symbol = stripped.split(maxsplit=1)
        except ValueError as err:
            raise SystemExit(f"invalid baseline entry: {raw!r}") from err
        entries.append((pathlib.Path(raw_path), symbol.strip()))
    return entries


def selector_parts(selector: str) -> tuple[str | None, str]:
    if "::" not in selector:
        return None, selector
    owner, symbol = selector.rsplit("::", maxsplit=1)
    return owner.strip(), symbol.strip()


def is_pub_trait_method(lines: list[str], line_no: int) -> bool:
    brace_depth = 0
    trait_depths: list[int] = []

    for raw in lines[: line_no - 1]:
        if PUB_TRAIT_RE.match(raw) and "{" in raw:
            trait_depths.append(brace_depth)
        opens = raw.count("{")
        closes = raw.count("}")
        brace_depth += opens - closes
        while trait_depths and brace_depth <= trait_depths[-1]:
            trait_depths.pop()
    return bool(trait_depths)


def enclosing_impl_owner(lines: list[str], line_no: int) -> str | None:
    brace_depth = 0
    impl_depths: list[tuple[int, str]] = []

    for raw in lines[: line_no - 1]:
        if IMPL_RE.match(raw) and "{" in raw:
            owner = impl_owner(raw)
            if owner is not None:
                impl_depths.append((brace_depth, owner))
        opens = raw.count("{")
        closes = raw.count("}")
        brace_depth += opens - closes
        while impl_depths and brace_depth <= impl_depths[-1][0]:
            impl_depths.pop()

    if not impl_depths:
        return None
    return impl_depths[-1][1]


def impl_owner(raw: str) -> str | None:
    if match := IMPL_TRAIT_OWNER_RE.match(raw):
        return match.group(1)
    if match := IMPL_OWNER_RE.match(raw):
        return match.group(1)
    return None


def doc_lines_before(lines: list[str], line_no: int) -> list[str]:
    docs: list[str] = []
    index = line_no - 2
    while index >= 0:
        raw = lines[index]
        stripped = raw.strip()
        if not stripped:
            index -= 1
            continue
        if DOC_RE.match(raw):
            docs.append(project_doc_line(raw))
            index -= 1
            continue
        if stripped.startswith("#["):
            index -= 1
            continue
        break
    docs.reverse()
    return docs


def project_doc_line(raw: str) -> str:
    stripped = raw.lstrip()
    if stripped.startswith("///"):
        return stripped[3:]
    if stripped.startswith("//!"):
        return stripped[3:]
    return stripped


def has_runnable_doctest(doc_lines: list[str]) -> bool:
    in_fence = False
    for raw in doc_lines:
        stripped = raw.strip()
        if not stripped.startswith("```"):
            continue
        if not in_fence:
            info = stripped[3:].strip().lower()
            if fence_is_runnable(info):
                return True
            in_fence = True
            continue
        in_fence = False
    return False


def fence_is_runnable(info: str) -> bool:
    if not info:
        return True
    tokens = {token for token in re.split(r"[\s,]+", info) if token}
    if tokens & NON_RUST_FENCE_TOKENS:
        return False
    known_rusty = {
        "rust",
        "should_panic",
        "edition2018",
        "edition2021",
        "edition2024",
    }
    return not tokens or tokens <= known_rusty or "rust" in tokens


def symbol_line_numbers(lines: list[str], selector: str) -> list[int]:
    owner, symbol = selector_parts(selector)
    out: list[int] = []
    for line_no, raw in enumerate(lines, start=1):
        match = PUB_FN_RE.match(raw)
        if match and match.group(1) == symbol:
            if owner is not None and enclosing_impl_owner(lines, line_no) != owner:
                continue
            out.append(line_no)
            continue
        if is_pub_trait_method(lines, line_no):
            trait_match = TRAIT_FN_RE.match(raw)
            if trait_match and trait_match.group(1) == symbol:
                if owner is not None and enclosing_impl_owner(lines, line_no) != owner:
                    continue
                out.append(line_no)
    return out


def main() -> int:
    root = repo_root()
    failures: list[str] = []

    for relative_path, symbol in load_baseline(root):
        path = root / relative_path
        lines = path.read_text().splitlines()
        matches = symbol_line_numbers(lines, symbol)
        if not matches:
            failures.append(f"{relative_path}: symbol `{symbol}` not found")
            continue
        if len(matches) > 1:
            failures.append(
                f"{relative_path}: symbol `{symbol}` is ambiguous; use `Type::method` to disambiguate"
            )
            continue
        doc_lines = doc_lines_before(lines, matches[0])
        if not has_runnable_doctest(doc_lines):
            failures.append(
                f"{relative_path}:{matches[0]}: `{symbol}` is missing a runnable doctest"
            )

    if not failures:
        return 0

    print(
        "ERROR: curated public API example baseline is missing runnable doctests.",
        file=sys.stderr,
    )
    print(file=sys.stderr)
    for failure in failures:
        print(f"  {failure}", file=sys.stderr)
    print(file=sys.stderr)
    print(
        "Keep `.public-api-examples.txt` intentionally small and add runnable examples to those symbols.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
