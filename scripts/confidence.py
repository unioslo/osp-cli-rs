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
import os
import shutil
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

MIRI_TOOLCHAIN = "nightly-2026-03-24"
TOOL_VERSIONS = {"cargo-audit": "0.22.2", "cargo-llvm-cov": "0.8.4"}
GENERIC_CHECKS = {"fmt", "fmt-fix", "clippy", "test", "build", "audit", "metadata"}


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


def cargo_miri_test_command(*target_args: str) -> list[str]:
    """Build a Miri test invocation for interpreter-friendly targets.

    Miri is intentionally kept on nightly and outside the pinned stable
    toolchain. The lane stays focused on core unit targets; spawned process,
    PTY, plugin-discovery, and broad filesystem behavior remain owned by the
    normal confidence lanes.
    """

    return ["cargo", f"+{MIRI_TOOLCHAIN}", "miri", "test", *target_args, "--locked"]


def clippy_command() -> list[str]:
    """Build the curated lint command used across local and CI workflows.

    The deny list is intentionally selective rather than `-D warnings`: it
    codifies the team's current "high-signal, low-noise" lint policy.
    """

    command = ["cargo", "clippy", "--locked", "--all-features", "--all-targets", "--"]
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
        command=["cargo", "doc", "--locked", "--no-deps"],
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
    miri_setup = ConfidenceCheck(
        name="miri-setup",
        description="Prepare the nightly sysroot used by cargo-miri.",
        command=["cargo", f"+{MIRI_TOOLCHAIN}", "miri", "setup"],
    )
    miri_smoke = ConfidenceCheck(
        name="miri-smoke",
        description="Focused interpreter check for Miri-safe fuzzy and render contracts.",
        command=cargo_miri_test_command("--test", "miri"),
        env={"MIRIFLAGS": "-Zmiri-disable-isolation"},
    )
    miri_config = ConfidenceCheck(
        name="miri-config",
        description="Config command unit tests under Miri.",
        command=cargo_miri_test_command("--lib", "cli::commands::config::tests"),
        env={"MIRIFLAGS": "-Zmiri-disable-isolation"},
    )
    miri_quick = ConfidenceCheck(
        name="miri-quick",
        description="DSL quick narrowing and envelope tests under Miri.",
        command=cargo_miri_test_command("--lib", "dsl::verbs::quick::tests"),
        env={"MIRIFLAGS": "-Zmiri-disable-isolation"},
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
    dependency_audit = ConfidenceCheck(
        name="audit",
        description="Fresh dependency advisory check against the committed lockfile.",
        command=["cargo", "audit", "--file", "Cargo.lock"],
    )
    wrapper = ConfidenceCheck(
        name="product-wrapper",
        description="Compile the copyable downstream wrapper as an external consumer.",
        command=[
            "cargo", "check", "--locked", "--manifest-path",
            str(root / "examples" / "product-wrapper" / "Cargo.toml"),
            "--target-dir", str(root / "target"),
        ],
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
    coverage_full = ConfidenceCheck(
        name="coverage",
        description="Full coverage gate.",
        command=[python, str(root / "scripts" / "coverage.py"), "gate"],
    )
    build = ConfidenceCheck(
        name="build",
        description="Build the release operator binary using the committed lockfile.",
        command=["cargo", "build", "--release", "--locked", "--bin", "osp"],
    )
    startup_budget = ConfidenceCheck(
        name="startup-budget",
        description="Check release startup latency and binary size budgets.",
        command=[python, str(root / "scripts" / "release-budget.py")],
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
        "miri": ConfidenceLane(
            name="miri",
            description=(
                "Nightly Miri check for interpreter-friendly core contracts."
            ),
            covers=(
                "fuzzy fallback contracts under Miri",
                "structured rendering contracts under Miri",
                "config command context and write-path unit tests under Miri",
                "DSL quick narrowing and envelope unit tests under Miri",
                "memory-model checks for core host logic",
            ),
            omits=(
                "spawned CLI behavior",
                "plugin subprocess behavior",
                "PTY behavior",
                "broad filesystem contracts",
                "coverage gates",
            ),
            checks=[miri_setup, miri_smoke, miri_config, miri_quick],
        ),
        "full": ConfidenceLane(
            name="full",
            description=(
                "Full confidence: docs, static checks, consumer build, advisories, doctests, and one instrumented test pass."
            ),
            covers=(
                "public docs contract",
                "static policy",
                "unit and doctest coverage",
                "visible CLI behavior",
                "in-process subsystem flows",
                "PTY behavior",
                "full coverage gate",
                "external product-wrapper build",
                "dependency advisory check",
                "release startup and binary size budgets",
            ),
            omits=(
                "crate publish dry-run",
                "release packaging",
                "UiO product tests (run in the paired product checkout)",
                "nightly Miri (separate lane)",
            ),
            checks=[
                public_docs,
                rustdoc_warnings,
                *static_checks,
                public_api_examples,
                doctests,
                wrapper,
                dependency_audit,
                coverage_full,
                build,
                startup_budget,
            ],
        ),
        "pre-push": ConfidenceLane(
            name="pre-push",
            description=(
                "Pre-push convenience: local docs, static checks and behavior boundaries."
            ),
            covers=(
                "public docs contract",
                "static policy",
                "visible CLI behavior",
                "in-process subsystem flows",
            ),
            omits=(
                "PTY behavior",
                "coverage (run the full lane for authoritative evidence)",
                "release build and quality budgets",
            ),
            checks=[
                public_docs,
                rustdoc_warnings,
                *static_checks,
                *behavior_checks,
            ],
        ),
    }


def check_catalog(root: Path, cwd: Path | None = None) -> dict[str, ConfidenceCheck]:
    """Expose lane checks and small standalone operations through one CLI."""

    python = sys.executable or "python3"
    checks = {
        check.name: check
        for lane in lane_catalog(root).values()
        for check in lane.checks
    }
    for name, description, command in (
        ("coverage-fast", "Validate a fast changed-file coverage report for review.", [
            python, str(root / "scripts" / "coverage.py"), "gate", "--fast",
        ]),
        ("e2e", "Run existing process and PTY contracts under isolated settings.", [
            python, str(root / "scripts" / "run-hermetic-cargo.py"), "--",
            *cargo_test_command("--test", "e2e"),
        ]),
        ("fmt-fix", "Format Rust source.", ["cargo", "fmt", "--all"]),
        ("metadata", "Describe the locked package and configured target directory.", [
            "cargo", "metadata", "--locked", "--no-deps", "--format-version", "1",
        ]),
        ("test", "Run existing tests under isolated runtime settings.", [
            python, str(root / "scripts" / "run-hermetic-cargo.py"),
            "--cwd", str(cwd or root), "--", *cargo_test_command("--all-features"),
        ]),
        ("coverage-summary", "Show the full instrumented coverage summary.", [
            python, str(root / "scripts" / "coverage.py"), "run",
            "--all-features", "--locked", "--summary-only",
        ]),
        ("coverage-baseline", "Capture an intentional coverage policy baseline.", [
            python, str(root / "scripts" / "coverage.py"), "baseline",
        ]),
        ("startup-baseline", "Capture an intentional release quality baseline.", [
            python, str(root / "scripts" / "release-budget.py"), "--baseline",
        ]),
        ("publish-dry-run", "Verify the packaged crate without publication.", [
            "cargo", "publish", "--dry-run", "--locked",
        ]),
    ):
        checks[name] = ConfidenceCheck(name, description, command)
    return checks


def required_tools(checks: list[ConfidenceCheck], root: Path) -> set[str]:
    """Collect prerequisites before a lane spends time building anything."""

    special = {
        "contract-env": {"bash", "rg"},
        "public-api-examples": set(),
        "startup-budget": set(),
        "startup-baseline": set(),
        "fmt": {"cargo", "rustfmt"},
        "fmt-fix": {"cargo", "rustfmt"},
        "clippy": {"cargo", "clippy"},
        "audit": {"cargo", "cargo-audit"},
        "metadata": {"cargo"},
    }
    tools: set[str] = set()
    for check in checks:
        tools.update(special.get(check.name, {"cargo"}))
        if check.name.startswith("coverage"):
            tools.update({"cargo-llvm-cov", "llvm-tools-preview"})
        if check.name.startswith("miri"):
            tools.add("miri")
    # The committed native GNU/Linux target configuration requires this linker.
    config = root / ".cargo" / "config.toml"
    if any(check.name not in special or check.name == "clippy" for check in checks):
        if sys.platform.startswith("linux") and os.uname().machine == "x86_64":
            if config.exists() and "-fuse-ld=lld" in config.read_text():
                tools.add("ld.lld")
    return tools


def tool_available(tool: str, root: Path | None = None) -> bool:
    """Check tools without invoking a dependency build or changing files."""

    if tool == "llvm-tools-preview":
        if shutil.which("rustc") is None:
            return False
        result = subprocess.run(
            ["rustc", "--print", "target-libdir"], cwd=root,
            capture_output=True, text=True, check=False,
        )
        if result.returncode or not result.stdout.strip():
            return False
        tool_dir = Path(result.stdout.strip()).parent / "bin"
        suffix = ".exe" if os.name == "nt" else ""
        return all(
            os.access(tool_dir / f"{name}{suffix}", os.X_OK)
            for name in ("llvm-cov", "llvm-profdata")
        )

    commands = {
        "rustfmt": ["cargo", "fmt", "--version"],
        "clippy": ["cargo", "clippy", "--version"],
        "miri": ["cargo", f"+{MIRI_TOOLCHAIN}", "miri", "--version"],
        "cargo-audit": ["cargo-audit", "--version"],
        "cargo-llvm-cov": ["cargo-llvm-cov", "llvm-cov", "--version"],
    }
    command = commands.get(tool)
    if command is None:
        return shutil.which(tool) is not None
    if shutil.which(command[0]) is None:
        return False
    if tool in TOOL_VERSIONS and shutil.which("cargo") is None:
        return False
    result = subprocess.run(command, cwd=root, capture_output=True, text=True, check=False)
    if result.returncode:
        return False
    version = TOOL_VERSIONS.get(tool)
    return version is None or f"{tool} {version}" in result.stdout.splitlines()


def preflight(checks: list[ConfidenceCheck], root: Path) -> None:
    """Fail with all missing prerequisites before the first expensive check."""

    missing = [tool for tool in sorted(required_tools(checks, root)) if not tool_available(tool, root)]
    if missing:
        fail(
            "Missing or unsupported confidence tools: " + ", ".join(missing)
            + ". Install pinned helpers with: python3 scripts/confidence.py --install-tools full. "
            + "Install toolchain components with rustup; GNU/Linux builds require ld.lld (package lld)."
        )


def install_tools(checks: list[ConfidenceCheck], root: Path) -> None:
    """Ensure active LLVM components and pinned helpers for the selected lane."""

    if shutil.which("cargo") is None:
        fail("cargo is required to install confidence tools.")
    tools = required_tools(checks, root)
    if "llvm-tools-preview" in tools and not tool_available("llvm-tools-preview", root):
        if shutil.which("rustup") is None:
            fail("rustup is required to install llvm-tools-preview for the active toolchain.")
        run_check(root, ConfidenceCheck(
            "install-llvm-tools", "Install the active toolchain's LLVM coverage tools.",
            ["rustup", "component", "add", "llvm-tools-preview"],
        ))
        if not tool_available("llvm-tools-preview", root):
            fail("llvm-tools-preview did not provide active llvm-cov and llvm-profdata binaries.")
    for tool in sorted(tools & TOOL_VERSIONS.keys()):
        if not tool_available(tool, root):
            run_check(root, ConfidenceCheck(
                f"install-{tool}", f"Install {tool} {TOOL_VERSIONS[tool]}.",
                ["cargo", "install", "--locked", "--version", TOOL_VERSIONS[tool], tool],
            ))


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


def run_check(root: Path, check: ConfidenceCheck, *, to_stderr: bool = False) -> CheckResult:
    """Run one check and fail fast with a labeled error.

    Confidence lanes are operational guardrails. Once one check fails, more
    output is usually noise rather than signal.
    """

    stream = sys.stderr if to_stderr else sys.stdout
    print(f"\n==> [{check.name}] {check.description}", file=stream, flush=True)
    started = time.perf_counter()
    env = None
    if check.env:
        env = dict(**os.environ, **check.env)
    result = subprocess.run(check.command, cwd=root, env=env)
    elapsed = time.perf_counter() - started
    if result.returncode != 0:
        print(
            f"\nConfidence failed at [{check.name}] after {elapsed:.1f}s.",
            file=sys.stderr,
        )
        raise SystemExit(result.returncode)
    print(f"    completed in {elapsed:.1f}s", file=stream, flush=True)
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
        help="Lane to run (default: local). Use --list to see lanes and checks.",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        help="List available confidence lanes and named checks, then exit.",
    )
    parser.add_argument("--check", help="Run one named check instead of a lane.")
    parser.add_argument(
        "--cwd", type=Path,
        help="Repository for a single generic check; lanes stay anchored to the framework.",
    )
    parser.add_argument(
        "--install-tools", nargs="?", const="full", metavar="LANE",
        help="Install pinned Cargo helpers for a lane (default: full), then exit.",
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
        print("Available named checks:")
        for check in check_catalog(root).values():
            print(f"  - {check.name}: {check.description}")
        return

    if args.check:
        if args.lane or args.install_tools:
            parser.error("--check cannot be combined with a lane or --install-tools")
        if args.cwd and args.check not in GENERIC_CHECKS:
            parser.error("--cwd is only supported with " + ", ".join(sorted(GENERIC_CHECKS)))
        cwd = (args.cwd or root).resolve()
        if not cwd.is_dir():
            parser.error(f"check directory does not exist: {cwd}")
        checks = check_catalog(root, cwd)
        check = checks.get(args.check)
        if check is None:
            parser.error(f"unknown check: {args.check}; choose one of: {', '.join(sorted(checks))}")
        preflight([check], cwd)
        run_check(cwd, check, to_stderr=True)
        return

    if args.cwd:
        parser.error("--cwd requires --check")
    if args.install_tools and args.lane:
        parser.error("choose a lane with --install-tools LANE")

    lane_name = args.install_tools or args.lane or "local"
    lane = lanes.get(lane_name)
    if lane is None:
        fail(
            f"unknown confidence lane: {lane_name}. "
            f"Choose one of: {', '.join(sorted(lanes))}"
        )
    if args.install_tools:
        install_tools(lane.checks, root)
        return

    print_lane_summary(lane)
    preflight(lane.checks, root)
    results: list[CheckResult] = []
    started = time.perf_counter()
    for check in lane.checks:
        results.append(run_check(root, check))
    total = time.perf_counter() - started
    render_results(lane, results, total=total)


if __name__ == "__main__":
    main()
