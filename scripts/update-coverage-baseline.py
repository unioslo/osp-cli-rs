#!/usr/bin/env python3
from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
from datetime import date
from pathlib import Path


def fail(message: str) -> None:
    print(message, file=sys.stderr)
    raise SystemExit(1)


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
            "`cargo llvm-cov` is required to update the coverage baseline.\n"
            "Install it with: cargo install cargo-llvm-cov --locked"
        )


def parse_overall_line_percent(report_path: Path) -> float:
    with report_path.open() as handle:
        report = json.load(handle)
    totals = report["data"][0]["totals"]["lines"]
    if "percent" in totals:
        return float(totals["percent"])
    count = totals.get("count", 0)
    covered = totals.get("covered", count)
    return 100.0 if count == 0 else (100.0 * covered / count)


def main() -> None:
    repo_root = Path(__file__).resolve().parent.parent
    baseline_path = repo_root / ".coverage-baseline.json"
    if not baseline_path.exists():
        fail("Missing .coverage-baseline.json.")

    ensure_coverage_tooling()

    with baseline_path.open() as handle:
        baseline = json.load(handle)
    previous = float(baseline["overall_line_percent"])

    with tempfile.TemporaryDirectory(prefix="osp-cov-baseline-") as tmp_dir:
        report_path = Path(tmp_dir) / "coverage.json"
        print("Running full root-package coverage for baseline update...")
        subprocess.run(
            [
                "cargo",
                "llvm-cov",
                "--all-features",
                "--json",
                f"--output-path={report_path}",
            ],
            cwd=repo_root,
            check=True,
        )
        current = parse_overall_line_percent(report_path)

    baseline["generated_at"] = date.today().isoformat()
    baseline["overall_line_percent"] = round(current, 2)

    with baseline_path.open("w") as handle:
        json.dump(baseline, handle, indent=2)
        handle.write("\n")

    print(
        f"Updated baseline overall line coverage: {previous:.2f}% -> {current:.2f}%"
    )
    print("Review the change and commit it deliberately when you want to raise the floor.")


if __name__ == "__main__":
    main()
