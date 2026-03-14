#!/usr/bin/env python3
"""Named local confidence lanes for osp-cli-rust.

This script is intentionally a thin orchestration layer over a small number of
explicit checks. The project testing strategy is behavior-first: contracts and
integration own most user-visible promises, `e2e` stays small and PTY-focused,
and coverage is a backstop rather than the main confidence signal. The lane
definitions here are the operational expression of that strategy.

Why this script exists instead of a pile of shell aliases:

- hooks, local workflows, CI, and release checks need the same lane names
- contributors need a short summary of what each lane covers and omits
- failures should stop at the first broken contract with a clear label

The important constraint is that this file should stay boring. Resist turning it
into a generic workflow engine, dynamic planner, or smart "run only what seems
necessary" tool. The lane table is meant to be easy to audit in review. If a
lane changes, the command list should change in one obvious place and the docs
should be updated to match.

Warnings for future edits:

- keep lane names stable unless hooks, docs, and CI are updated together
- keep coverage policy in `coverage.py`; do not re-implement coverage heuristics
  here
- prefer explicit command lists over conditionals that make the lane behavior
  hard to predict
- be cautious with parallel execution; stable ordering and readable failure
  output matter more here than shaving a few seconds
"""

from __future__ import annotations

import argparse
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path


CLIPPY_DENIES = (
    "clippy::collapsible_else_if",
    "clippy::collapsible_if",
    "clippy::derivable_impls",
    "clippy::get_first",
    "clippy::io_other_error",
    "clippy::lines_filter_map_ok",
    "clippy::manual_pattern_char_comparison",
    "clippy::match_like_matches_macro",
    "clippy::needless_as_bytes",
    "clippy::needless_borrow",
    "clippy::question_mark",
    "clippy::redundant_closure",
    "clippy::unnecessary_lazy_evaluations",
)


@dataclass(frozen=True)
class ConfidenceCheck:
    """A single named command that contributes one confidence signal."""

    name: str
    description: str
    command: list[str]
    env: dict[str, str] | None = None


@dataclass(frozen=True)
class ConfidenceLane:
    """A documented bundle of checks used by hooks, humans, and CI."""

    name: str
    description: str
    covers: tuple[str, ...]
    omits: tuple[str, ...]
    checks: list[ConfidenceCheck]


@dataclass(frozen=True)
class CheckResult:
    """Timing information for one successful check execution."""

    check: ConfidenceCheck
    elapsed_seconds: float


def fail(message: str) -> None:
    print(message, file=sys.stderr)
    raise SystemExit(1)


def repo_root() -> Path:
    """Anchor commands to the repository even when invoked from another cwd."""

    return Path(__file__).resolve().parent.parent


def cargo_test_command(*target_args: str) -> list[str]:
    """Build a locked `cargo test` invocation for one target slice.

    Keeping this shape in one helper reduces drift across lanes when the project
    changes its common cargo flags.
    """

    return ["cargo", "test", *target_args, "--locked"]


def clippy_command() -> list[str]:
    """Build the curated lint command used across local and CI workflows.

    The deny list is intentionally selective rather than `-D warnings`: it
    codifies the team's current "high-signal, low-noise" lint policy.
    """

    command = ["cargo", "clippy", "--all-features", "--all-targets", "--"]
    for lint in CLIPPY_DENIES:
        command.extend(["-D", lint])
    return command


def lane_catalog(root: Path) -> dict[str, ConfidenceLane]:
    """Return the project's named confidence lanes.

    This is kept as explicit data rather than assembled indirectly so reviewers
    can answer "what does this lane run?" without executing the script.
    """

    python = sys.executable or "python3"
    hermetic_runner = [python, str(root / "scripts" / "run-hermetic-cargo.py"), "--"]

    public_docs = ConfidenceCheck(
        name="public-docs",
        description="Repo-wide public Rustdoc coverage and feature-gate contract.",
        command=[python, str(root / "scripts" / "public-docs.py")],
    )
    rustdoc_warnings = ConfidenceCheck(
        name="rustdoc-warnings",
        description="Fail on rustdoc warnings such as broken intra-doc links.",
        command=["cargo", "doc", "--no-deps"],
        env={"RUSTDOCFLAGS": "-D warnings"},
    )
    public_api_examples = ConfidenceCheck(
        name="public-api-examples",
        description="Curated runnable doctest baseline for public entrypoints.",
        command=[python, str(root / "scripts" / "public-api-examples.py")],
    )
    contract_env = ConfidenceCheck(
        name="contract-env",
        description="Hermetic contract test environment guardrail.",
        command=[str(root / "scripts" / "check-contract-env.sh")],
    )
    fmt = ConfidenceCheck(
        name="fmt",
        description="Rust formatting check.",
        command=["cargo", "fmt", "--all", "--check"],
    )
    clippy = ConfidenceCheck(
        name="clippy",
        description="Fast lint and static correctness checks.",
        command=clippy_command(),
    )
    architecture = ConfidenceCheck(
        name="architecture",
        description="Architecture guardrail tests.",
        command=cargo_test_command("--test", "architecture"),
    )
    unit = ConfidenceCheck(
        name="unit",
        description="Internal lib/bin unit tests plus the root unit target.",
        command=cargo_test_command("--lib", "--bins", "--test", "unit"),
    )
    doctests = ConfidenceCheck(
        name="doctests",
        description="Public doctest and example coverage.",
        command=cargo_test_command("--doc"),
    )
    contracts = ConfidenceCheck(
        name="contracts",
        description="Spawned-binary CLI behavior contracts.",
        command=[*hermetic_runner, *cargo_test_command("--test", "contracts")],
    )
    integration = ConfidenceCheck(
        name="integration",
        description="In-process cross-subsystem behavior flows.",
        command=[*hermetic_runner, *cargo_test_command("--test", "integration")],
    )
    e2e = ConfidenceCheck(
        name="e2e",
        description="Real process and PTY behavior checks.",
        command=cargo_test_command("--test", "e2e"),
    )
    coverage_fast = ConfidenceCheck(
        name="coverage-fast",
        description="Approximate local coverage guardrail.",
        command=[python, str(root / "scripts" / "coverage.py"), "gate", "--fast"],
    )
    coverage_full = ConfidenceCheck(
        name="coverage",
        description="Full coverage gate.",
        command=[python, str(root / "scripts" / "coverage.py"), "gate"],
    )

    static_checks = [contract_env, fmt, clippy, architecture]
    behavior_checks = [contracts, integration]

    return {
        "static": ConfidenceLane(
            name="static",
            description=(
                "Fast static hygiene and structural policy checks."
            ),
            covers=(
                "formatting and lint policy",
                "hermetic contract environment",
                "architecture guardrails",
            ),
            omits=(
                "public docs contract",
                "spawned CLI behavior",
                "integration flows",
                "PTY behavior",
                "coverage gates",
            ),
            checks=static_checks,
        ),
        "local": ConfidenceLane(
            name="local",
            description=(
                "Fastest useful local confidence loop: docs, static checks, contracts, and integration."
            ),
            covers=(
                "public docs contract",
                "static policy",
                "visible CLI behavior",
                "in-process subsystem flows",
            ),
            omits=(
                "PTY behavior",
                "full unit sweep",
                "doctests",
                "coverage gates",
            ),
            checks=[public_docs, rustdoc_warnings, *static_checks, *behavior_checks],
        ),
        "behavior": ConfidenceLane(
            name="behavior",
            description=(
                "Behavior-focused lane: contracts and integration without PTY-heavy e2e."
            ),
            covers=(
                "visible CLI behavior",
                "in-process subsystem flows",
            ),
            omits=(
                "static policy",
                "PTY behavior",
                "coverage gates",
            ),
            checks=behavior_checks,
        ),
        "full": ConfidenceLane(
            name="full",
            description=(
                "Full local confidence: docs, static checks, unit coverage, behavior lanes, e2e, and full coverage."
            ),
            covers=(
                "public docs contract",
                "static policy",
                "unit and doctest coverage",
                "visible CLI behavior",
                "in-process subsystem flows",
                "PTY behavior",
                "full coverage gate",
            ),
            omits=(
                "crate publish dry-run",
                "release packaging",
            ),
            checks=[
                public_docs,
                rustdoc_warnings,
                *static_checks,
                unit,
                public_api_examples,
                doctests,
                *behavior_checks,
                e2e,
                coverage_full,
            ],
        ),
        "pre-push": ConfidenceLane(
            name="pre-push",
            description=(
                "Merge-guard approximation: local lane plus fast changed-file coverage."
            ),
            covers=(
                "public docs contract",
                "static policy",
                "visible CLI behavior",
                "in-process subsystem flows",
                "fast coverage approximation",
            ),
            omits=(
                "PTY behavior",
                "full release-path coverage",
            ),
            checks=[
                public_docs,
                rustdoc_warnings,
                *static_checks,
                *behavior_checks,
                coverage_fast,
            ],
        ),
    }


def print_lane_summary(lane: ConfidenceLane) -> None:
    """Render the lane contract before execution.

    The summary is there to make omissions visible, not just to advertise what
    runs. That reduces accidental misuse of the faster lanes.
    """

    print(f"Confidence lane: {lane.name}")
    print(f"Purpose: {lane.description}")
    print("Covers:")
    for item in lane.covers:
        print(f"  - {item}")
    print("Omits:")
    for item in lane.omits:
        print(f"  - {item}")
    print("Checks:")
    for index, check in enumerate(lane.checks, start=1):
        print(f"  {index}. {check.name}: {check.description}")


def run_check(root: Path, check: ConfidenceCheck) -> CheckResult:
    """Run one check and fail fast with a labeled error.

    Confidence lanes are operational guardrails. Once one check fails, more
    output is usually noise rather than signal.
    """

    print(f"\n==> [{check.name}] {check.description}", flush=True)
    started = time.perf_counter()
    env = None
    if check.env:
        env = dict(**subprocess.os.environ, **check.env)
    result = subprocess.run(check.command, cwd=root, env=env)
    elapsed = time.perf_counter() - started
    if result.returncode != 0:
        print(
            f"\nConfidence failed at [{check.name}] after {elapsed:.1f}s.",
            file=sys.stderr,
        )
        raise SystemExit(result.returncode)
    print(f"    completed in {elapsed:.1f}s", flush=True)
    return CheckResult(check=check, elapsed_seconds=elapsed)


def render_results(lane: ConfidenceLane, results: list[CheckResult], *, total: float) -> None:
    """Print a compact success summary after a lane completes."""

    print(f"\nConfidence OK: {lane.name} lane completed in {total:.1f}s")
    for result in results:
        print(f"  - {result.check.name}: {result.elapsed_seconds:.1f}s")


def build_parser() -> argparse.ArgumentParser:
    """Build the small CLI surface for named lane execution."""

    parser = argparse.ArgumentParser(
        description="Run named local confidence lanes for osp-cli-rust."
    )
    parser.add_argument(
        "lane",
        nargs="?",
        default="local",
        help="Lane to run. Use --list to see available lanes.",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        help="List available confidence lanes and exit.",
    )
    return parser


def main() -> None:
    """Parse CLI arguments and run the selected lane."""

    root = repo_root()
    lanes = lane_catalog(root)
    parser = build_parser()
    args = parser.parse_args()

    if args.list:
        print("Available confidence lanes:")
        for lane in lanes.values():
            print(f"  - {lane.name}: {lane.description}")
        return

    lane = lanes.get(args.lane)
    if lane is None:
        fail(
            f"unknown confidence lane: {args.lane}. "
            f"Choose one of: {', '.join(sorted(lanes))}"
        )

    print_lane_summary(lane)
    results: list[CheckResult] = []
    started = time.perf_counter()
    for check in lane.checks:
        results.append(run_check(root, check))
    total = time.perf_counter() - started
    render_results(lane, results, total=total)


if __name__ == "__main__":
    main()
