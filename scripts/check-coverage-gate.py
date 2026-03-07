#!/usr/bin/env python3
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


def fail(message: str) -> None:
    print(message, file=sys.stderr)
    raise SystemExit(1)


def run(*args: str, cwd: Path) -> str:
    result = subprocess.run(
        list(args),
        cwd=cwd,
        check=True,
        text=True,
        capture_output=True,
    )
    return result.stdout


def maybe_run(*args: str, cwd: Path) -> str | None:
    result = subprocess.run(
        list(args),
        cwd=cwd,
        check=False,
        text=True,
        capture_output=True,
    )
    if result.returncode != 0:
        return None
    return result.stdout


def resolve_repo_root() -> Path:
    return Path(
        run("git", "rev-parse", "--show-toplevel", cwd=Path.cwd()).strip()
    ).resolve()


def ensure_coverage_tooling() -> None:
    if shutil.which("cargo") is None:
        fail("`cargo` was not found in PATH.")
    result = subprocess.run(
        ["cargo", "llvm-cov", "--version"],
        check=False,
        text=True,
        capture_output=True,
    )
    if result.returncode != 0:
        fail(
            "`cargo llvm-cov` is required for the coverage gate.\n"
            "Install it with: cargo install cargo-llvm-cov --locked"
        )


def load_baseline(path: Path) -> dict[str, float]:
    with path.open() as handle:
        payload = json.load(handle)
    return {
        "overall_line_percent": float(payload["overall_line_percent"]),
        "changed_file_min_line_percent": float(payload["changed_file_min_line_percent"]),
        "min_executable_lines": float(payload.get("min_executable_lines", 0)),
    }


def resolve_diff_range(repo_root: Path) -> list[str]:
    upstream = maybe_run(
        "git",
        "rev-parse",
        "--abbrev-ref",
        "--symbolic-full-name",
        "@{upstream}",
        cwd=repo_root,
    )
    if upstream and upstream.strip():
        return [f"{upstream.strip()}..HEAD"]
    return ["--root", "HEAD"]


def changed_source_files(repo_root: Path) -> list[str]:
    diff_args = resolve_diff_range(repo_root)
    output = run(
        "git",
        "diff",
        "--name-only",
        "--diff-filter=ACMR",
        *diff_args,
        cwd=repo_root,
    )
    changed = []
    for raw in output.splitlines():
        path = raw.strip()
        if not path.endswith(".rs"):
            continue
        if not path.startswith("crates/"):
            continue
        if "/src/" not in path:
            continue
        changed.append(path)
    return sorted(set(changed))


def parse_report(report_path: Path, repo_root: Path) -> tuple[float, dict[str, dict[str, float]]]:
    with report_path.open() as handle:
        report = json.load(handle)

    data = report["data"][0]
    totals = data["totals"]["lines"]
    if "percent" in totals:
        overall = float(totals["percent"])
    else:
        count = totals.get("count", 0)
        covered = totals.get("covered", count)
        overall = 100.0 if count == 0 else (100.0 * covered / count)

    files: dict[str, dict[str, float]] = {}
    for entry in data.get("files", []):
        filename = Path(entry["filename"]).resolve()
        try:
            rel = filename.relative_to(repo_root).as_posix()
        except ValueError:
            rel = os.path.normpath(entry["filename"])
        lines = entry.get("summary", {}).get("lines", {})
        count = float(lines.get("count", 0))
        if "percent" in lines:
            percent = float(lines["percent"])
        else:
            covered = float(lines.get("covered", count))
            percent = 100.0 if count == 0 else (100.0 * covered / count)
        files[rel] = {"percent": percent, "count": count}

    return overall, files


def run_coverage(repo_root: Path, report_path: Path) -> None:
    print("Running full workspace coverage...")
    subprocess.run(
        [
            "cargo",
            "llvm-cov",
            "--workspace",
            "--all-features",
            "--json",
            f"--output-path={report_path}",
        ],
        cwd=repo_root,
        check=True,
    )


def main() -> None:
    repo_root = resolve_repo_root()
    ensure_coverage_tooling()
    baseline_path = repo_root / ".coverage-baseline.json"
    if not baseline_path.exists():
        fail(
            "Missing .coverage-baseline.json. Add a baseline before enabling the coverage gate."
        )

    baseline = load_baseline(baseline_path)
    changed_files = changed_source_files(repo_root)

    with tempfile.TemporaryDirectory(prefix="osp-cov-") as tmp_dir:
        report_path = Path(tmp_dir) / "coverage.json"
        run_coverage(repo_root, report_path)
        overall, files = parse_report(report_path, repo_root)

    errors: list[str] = []
    notes: list[str] = []

    baseline_overall = baseline["overall_line_percent"]
    if overall + 1e-9 < baseline_overall:
        errors.append(
            f"overall line coverage regressed: baseline={baseline_overall:.2f}% current={overall:.2f}%"
        )

    min_file_percent = baseline["changed_file_min_line_percent"]
    min_executable_lines = baseline["min_executable_lines"]
    for path in changed_files:
        entry = files.get(path)
        if entry is None:
            errors.append(f"no coverage entry found for changed source file: {path}")
            continue
        if entry["count"] < min_executable_lines:
            notes.append(
                f"skipping tiny file coverage gate for {path} ({entry['count']:.0f} executable lines)"
            )
            continue
        if entry["percent"] + 1e-9 < min_file_percent:
            errors.append(
                f"changed file below {min_file_percent:.1f}%: {path} ({entry['percent']:.2f}%)"
            )

    if errors:
        print("\nCoverage gate failed:\n", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        if notes:
            print("\nNotes:", file=sys.stderr)
            for note in notes:
                print(f"  - {note}", file=sys.stderr)
        raise SystemExit(1)

    print(
        f"Coverage OK: overall {overall:.2f}% (baseline {baseline_overall:.2f}%)"
    )
    if changed_files:
        print(
            f"Checked changed source files against {min_file_percent:.1f}% minimum: {len(changed_files)} file(s)"
        )
    else:
        print("No changed source files in push range; checked overall coverage only.")
    for note in notes:
        print(f"Note: {note}")


if __name__ == "__main__":
    main()
