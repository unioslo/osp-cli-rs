#!/usr/bin/env python3
"""Public Rustdoc contract checks for osp-cli-rust.

This script enforces two rules on the public library surface:

- exported public items must satisfy the compiler-backed `missing_docs`
  contract
- `#[cfg(feature = "...")]` public functions and trait methods should surface
  the agreed feature-gate prose in that Rustdoc

It exists separately from `confidence.py` because the policy has two distinct
operating modes:

- `--staged` for fast pre-commit checks against the staged blob
- repo-wide mode for local confidence lanes and CI

That split is intentional. The hook should judge what is actually being
committed, not whatever happens to be in the working tree at the moment. The
repo-wide mode exists so the same contract can be enforced outside hooks.

The compiler is the authority for the actual public-item coverage rule. This
script deliberately keeps only the extra feature-gate wording policy on a
lightweight syntax heuristic. Do not casually grow that heuristic into a full
Rust visibility/export checker; when parity with the compiler matters, prefer
running the compiler.

Warnings for future edits:

- preserve the staged-index behavior; reading from the working tree weakens the
  hook contract
- keep the compiler-backed missing-docs check aligned with the library crate
  rather than inventing a second export-graph model here
- keep test modules out of scope for the feature-gate prose heuristic so
  internal helper churn does not become docs noise
"""

from __future__ import annotations

import argparse
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile


PUB_FN_RE = re.compile(
    r"^\s*pub\s+(?:const\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\b"
)
TRAIT_FN_RE = re.compile(
    r"^\s*(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\b"
)
PUB_TRAIT_RE = re.compile(r"^\s*pub\s+trait\s+([A-Za-z_][A-Za-z0-9_]*)\b")
ATTR_RE = re.compile(r"^\s*#\s*\[")
DOC_RE = re.compile(r"^\s*(///|//!\s*|#\s*\[\s*doc\s*=)")
DOC_ATTR_RE = re.compile(r'^\s*#\s*\[\s*doc\s*=\s*"(.*)"\s*\]\s*$')
CFG_FEATURE_RE = re.compile(
    r'^\s*#\s*\[\s*cfg\s*\(\s*feature\s*=\s*"([^"]+)"\s*\)\s*\]\s*$'
)

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


def staged_rust_files(root: pathlib.Path) -> list[pathlib.Path]:
    """Return staged production Rust files under `src/`."""

    cmd = [
        "git",
        "diff",
        "--cached",
        "--name-only",
        "--diff-filter=ACMR",
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
    return sorted(
        pathlib.Path(raw.strip())
        for raw in result.stdout.splitlines()
        if raw.strip() and not is_internal_test_module(pathlib.Path(raw.strip()))
    )


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


def doc_context(lines: list[str], line_no: int) -> tuple[list[str], list[str]]:
    """Collect doc text and directly attached `cfg(feature = ...)` gates."""

    docs: list[str] = []
    features: list[str] = []
    index = line_no - 2

    while index >= 0:
        raw = lines[index]
        stripped = raw.strip()
        if not stripped:
            index -= 1
            continue
        if DOC_RE.match(raw):
            docs.append(doc_text_from_line(raw))
            index -= 1
            continue
        if ATTR_RE.match(raw):
            if match := CFG_FEATURE_RE.match(raw):
                features.append(match.group(1))
            index -= 1
            continue
        break

    docs.reverse()
    features.reverse()
    return docs, features


def doc_text_from_line(raw: str) -> str:
    """Project one doc-comment line into plain text for phrase checks."""

    stripped = raw.lstrip()
    if stripped.startswith("///"):
        return stripped[3:].strip()
    if stripped.startswith("//!"):
        return stripped[3:].strip()
    if match := DOC_ATTR_RE.match(raw):
        return match.group(1)
    return stripped


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


def feature_gate_doc_failures_in_lines(
    path: pathlib.Path,
    lines: list[str],
) -> list[str]:
    """Return feature-gate prose failures in the given lines."""

    failures: list[str] = []
    for line_no, line in enumerate(lines, start=1):
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
        if not has_rustdoc(lines, line_no):
            continue
        docs, features = doc_context(lines, line_no)
        for feature in features:
            required = feature_gate_phrase(feature)
            if required not in " ".join(docs):
                failures.append(
                    f"{path}:{line_no}: public function `{name}` is gated by "
                    f'feature `{feature}` but its Rustdoc is missing the prose '
                    f'"{required}"'
                )
    return failures


def feature_gate_phrase(feature: str) -> str:
    """Return the required narrow prose pattern for a cargo feature gate."""

    return f"Only available with the `{feature}` cargo feature"


def merged_rustflags(current: str | None, extra: str) -> str:
    """Append one rustflag while preserving any existing caller-provided flags."""

    if not current:
        return extra
    return f"{current} {extra}"


def compiler_missing_docs_failures(root: pathlib.Path, cwd: pathlib.Path) -> list[str]:
    """Return compiler-backed missing-docs failures for the library crate."""

    env = os.environ.copy()
    env["RUSTFLAGS"] = merged_rustflags(env.get("RUSTFLAGS"), "-Dmissing-docs")
    env.setdefault("CARGO_TARGET_DIR", str(root / "target" / "public-docs"))
    result = subprocess.run(
        [
            "cargo",
            "check",
            "--lib",
            "--locked",
            "--message-format",
            "short",
        ],
        cwd=cwd,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode == 0:
        return []

    diagnostics = [
        line
        for line in (result.stdout.splitlines() + result.stderr.splitlines())
        if line.strip()
    ]
    if not diagnostics:
        diagnostics = ["`cargo check --lib --locked` failed without diagnostics."]
    return [
        "compiler-backed public Rustdoc coverage check failed:",
        *[f"  {line}" for line in diagnostics],
    ]


def export_staged_index(root: pathlib.Path) -> pathlib.Path:
    """Export the staged git index into a temporary directory."""

    target_root = root / "target"
    target_root.mkdir(parents=True, exist_ok=True)
    temp_root = pathlib.Path(
        tempfile.mkdtemp(prefix="osp-public-docs-", dir=target_root)
    )
    subprocess.run(
        [
            "git",
            "checkout-index",
            "--all",
            "--force",
            f"--prefix={temp_root}{os.sep}",
        ],
        cwd=root,
        capture_output=True,
        text=True,
        check=True,
    )
    return temp_root


def rust_source_files(root: pathlib.Path) -> list[pathlib.Path]:
    """Return production Rust source files that participate in this contract."""

    return sorted(
        path.relative_to(root)
        for path in root.joinpath("src").rglob("*.rs")
        if not is_internal_test_module(path.relative_to(root))
    )


def check_workspace_public_docs(root: pathlib.Path) -> list[str]:
    """Evaluate the public docs contract across the whole repository tree."""

    failures = compiler_missing_docs_failures(root, root)
    for path in rust_source_files(root):
        lines = root.joinpath(path).read_text().splitlines()
        failures.extend(feature_gate_doc_failures_in_lines(path, lines))
    return failures


def check_staged_public_docs(root: pathlib.Path) -> list[str]:
    """Evaluate the public docs contract against the staged git index."""

    files = staged_rust_files(root)
    if not files:
        return []

    failures: list[str] = []
    staged_root = export_staged_index(root)
    try:
        failures.extend(compiler_missing_docs_failures(root, staged_root))
        for path in files:
            lines = staged_file_lines(root, path)
            if lines is None:
                continue
            failures.extend(feature_gate_doc_failures_in_lines(path, lines))
    finally:
        shutil.rmtree(staged_root, ignore_errors=True)
    return failures


def main() -> int:
    """Parse CLI mode and run the public docs contract check."""

    parser = argparse.ArgumentParser(
        description=(
            "Check public Rustdoc coverage and required feature-gate notes."
        )
    )
    parser.add_argument(
        "--staged",
        action="store_true",
        help="Check the staged git index, intended for pre-commit hooks.",
    )
    args = parser.parse_args()

    root = repo_root()
    if args.staged:
        failures = check_staged_public_docs(root)
    else:
        failures = check_workspace_public_docs(root)
    if not failures:
        return 0

    if args.staged:
        print(
            "ERROR: staged public Rust items must satisfy the public Rustdoc contract.",
            file=sys.stderr,
        )
    else:
        print(
            "ERROR: public Rust items must satisfy the public Rustdoc contract.",
            file=sys.stderr,
        )
    print(file=sys.stderr)
    for failure in failures:
        print(f"  {failure}", file=sys.stderr)
    print(file=sys.stderr)
    print(
        "Add a `///` Rustdoc comment above the exported item, and include the "
        "standard feature-gate prose when required.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
