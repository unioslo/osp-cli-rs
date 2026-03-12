#!/usr/bin/env python3
"""Coverage utilities and policy enforcement for osp-cli-rust.

Coverage is intentionally not the primary testing strategy in this repository.
Behavior-first tests at the contract, integration, and carefully limited `e2e`
layers carry most of the confidence. This script exists to provide a backstop:

- a full gate that prevents the checked-in overall floor from regressing
- a changed-file rule that catches obviously under-tested source changes
- a fast local approximation that is cheap enough for pre-push use

The design bias here is practical rather than theoretically perfect. The fast
gate is deliberately aligned with the non-PTY local lane instead of trying to
predict every test target that might contribute indirect coverage. The full gate
is the authority. Keep that distinction clear.

There are a few policy edges that are easy to "simplify" incorrectly:

- the full baseline floor should stay a deliberate checked-in policy file, not
  an auto-refreshed artifact
- fast mode should remain an approximation and should not silently grow into a
  second full CI lane
- declaration-only and tiny-file exemptions are pragmatic exceptions for files
  that do not produce useful llvm-cov output; do not broaden them casually
- coverage runs use an isolated temporary workspace on purpose to avoid crosstalk
  between local runs, hooks, and CI jobs

If this file becomes much smarter, it will likely become less trustworthy. Favor
explicit target sets, obvious failure messages, and heuristics that can be
explained in one paragraph.
"""

from __future__ import annotations

import argparse
import contextlib
import fcntl
import json
import os
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from datetime import date
from pathlib import Path
from typing import Any, Literal


RUNS_TO_KEEP = 4
JSON_REPORT_BASE_ARGS = ("--all-features", "--locked", "--json")
FAST_COVERAGE_TARGET_ARGS = (
    "--lib",
    "--bins",
    "--test",
    "contracts",
    "--test",
    "integration",
)
FULL_COVERAGE_TARGET_ARGS = (
    "--lib",
    "--bins",
    "--test",
    "unit",
    "--test",
    "contracts",
    "--test",
    "integration",
    "--test",
    "e2e",
)


@dataclass(frozen=True)
class CoveragePlan:
    """The coverage strategy chosen for one invocation."""

    mode: Literal["skip", "fast", "full"]
    display_diff_basis: str
    changed_files: list[str]
    target_args: tuple[str, ...]
    target_label: str
    reason: str


@dataclass(frozen=True)
class FileCoverage:
    """Line coverage summary for one source file."""

    percent: float
    count: float


@dataclass(frozen=True)
class CoverageReport:
    """Overall and per-file line coverage extracted from llvm-cov JSON."""

    overall: float
    files: dict[str, FileCoverage]


@dataclass(frozen=True)
class GateResult:
    """Coverage gate evaluation outcome after policy rules are applied."""

    overall: float | None
    files_considered: int
    files_checked: int
    files_skipped_by_policy: int
    errors: list[str]
    policy_notes: list[str]


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


def coverage_root() -> Path:
    """Return the shared temp root for isolated coverage workspaces."""

    return Path(tempfile.gettempdir()) / "osp-cli-cov"


def coverage_runs_root() -> Path:
    """Store recent run artifacts separately from the scratch tmp dir."""

    return coverage_root() / "runs"


def coverage_tmp_root() -> Path:
    """Use a dedicated temp dir so child processes do not pollute the system tmp."""

    return coverage_root() / "tmp"


def repo_root() -> Path:
    """Anchor the script to its own repository regardless of caller cwd."""

    return Path(__file__).resolve().parent.parent


@contextlib.contextmanager
def coverage_lock() -> object:
    """Serialize coverage workspace setup and teardown.

    The lock exists because cargo/llvm-cov and spawned test processes may share
    temp-space assumptions. We prefer a simple coarse lock over hard-to-debug
    temp directory races.
    """

    root = coverage_root()
    root.mkdir(parents=True, exist_ok=True)
    lock_path = root / ".lock"
    with lock_path.open("w") as handle:
        # POSIX-only lock is acceptable because local and CI coverage tooling runs on Unix.
        fcntl.flock(handle.fileno(), fcntl.LOCK_EX)
        yield


def _remove_path(path: Path) -> None:
    if path.is_dir() and not path.is_symlink():
        shutil.rmtree(path)
        return
    path.unlink(missing_ok=True)


def _clear_dir(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)
    for child in path.iterdir():
        _remove_path(child)


def clear_workspace_tmp() -> None:
    """Clear the shared scratch tmp directory between isolated runs."""

    _clear_dir(coverage_tmp_root())


def _prune_oldest(root: Path, keep: int) -> None:
    root.mkdir(parents=True, exist_ok=True)
    entries = sorted(
        root.iterdir(),
        key=lambda path: (path.stat().st_mtime, path.name),
    )
    while len(entries) > keep:
        oldest = entries.pop(0)
        _remove_path(oldest)


def prepare_workspace(prefix: str) -> tuple[Path, dict[str, str]]:
    """Prepare an isolated run directory and temp-oriented environment.

    The per-run directory is kept briefly for debugging; the temp directory is
    aggressively cleared because it is the most common source of interference.
    """

    runs_root = coverage_runs_root()
    tmp_root = coverage_tmp_root()

    runs_root.mkdir(parents=True, exist_ok=True)
    _clear_dir(tmp_root)

    run_dir = Path(tempfile.mkdtemp(prefix=prefix, dir=runs_root))
    _prune_oldest(runs_root, RUNS_TO_KEEP)

    env = os.environ.copy()
    for key in ("TMPDIR", "TMP", "TEMP", "TEMPDIR"):
        env[key] = str(tmp_root)
    return run_dir, env


@contextlib.contextmanager
def prepared_workspace(prefix: str) -> tuple[Path, dict[str, str]]:
    """Hold the coverage lock while a caller uses an isolated workspace."""

    with coverage_lock():
        run_dir, env = prepare_workspace(prefix)
        try:
            yield run_dir, env
        finally:
            clear_workspace_tmp()

def ensure_coverage_tooling(*, purpose: str) -> None:
    """Fail early with a useful install hint when cargo-llvm-cov is missing."""

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
            f"`cargo llvm-cov` is required to {purpose}.\n"
            "Install it with: cargo install cargo-llvm-cov --locked"
        )


def load_baseline(path: Path) -> dict[str, float]:
    """Load only the numeric policy values needed by the gate."""

    with path.open() as handle:
        payload = json.load(handle)
    return {
        "overall_line_percent": float(payload["overall_line_percent"]),
        "changed_file_min_line_percent": float(payload["changed_file_min_line_percent"]),
        "min_executable_lines": float(payload.get("min_executable_lines", 0)),
    }


def load_baseline_payload(path: Path) -> dict[str, Any]:
    """Load the full baseline payload for deliberate baseline updates."""

    with path.open() as handle:
        return json.load(handle)


def write_baseline_payload(path: Path, payload: dict[str, Any]) -> None:
    """Rewrite the baseline file in a stable, review-friendly format."""

    with path.open("w") as handle:
        json.dump(payload, handle, indent=2)
        handle.write("\n")


def resolve_branch_diff_args(repo_root: Path) -> tuple[list[str], str]:
    """Pick the branch comparison basis for local and hook-driven runs.

    The upstream branch is the most useful default for pre-push semantics. When
    no upstream exists, comparing against the root commit keeps the script usable
    in fresh or detached repos.
    """

    upstream = maybe_run(
        "git",
        "rev-parse",
        "--abbrev-ref",
        "--symbolic-full-name",
        "@{upstream}",
        cwd=repo_root,
    )
    if upstream and upstream.strip():
        diff_range = f"{upstream.strip()}..HEAD"
        return [diff_range], diff_range
    return ["--root", "HEAD"], "--root HEAD"


def is_internal_test_module(path: str) -> bool:
    """Treat inline test modules as test support, not changed production source."""

    if not path.endswith(".rs"):
        return False
    return path.endswith("/tests.rs") or "/tests/" in path


def parse_path_output(output: str | None) -> list[str]:
    """Normalize newline-delimited git path output into a stable unique list."""

    if not output:
        return []

    paths = []
    for raw in output.splitlines():
        path = raw.strip()
        if not path:
            continue
        paths.append(path)
    return sorted(set(paths))


def collect_changed_paths(repo_root: Path, diff_args: list[str]) -> list[str]:
    """Collect changed paths from one git diff invocation."""

    return parse_path_output(
        run(
            "git",
            "diff",
            "--name-only",
            "--diff-filter=ACMR",
            *diff_args,
            cwd=repo_root,
        )
    )


def collect_untracked_paths(repo_root: Path) -> list[str]:
    """Include untracked files so manual local runs see new source files too."""

    return parse_path_output(
        maybe_run(
            "git",
            "ls-files",
            "--others",
            "--exclude-standard",
            cwd=repo_root,
        )
    )


def collect_candidate_paths(repo_root: Path) -> tuple[list[str], str]:
    """Combine branch, index, worktree, and untracked changes.

    This is intentionally broader than strict pre-push semantics because manual
    local runs are more useful when they see not-yet-committed source changes.
    The printed basis string exists to make that broadened scope obvious.
    """

    branch_diff_args, branch_basis = resolve_branch_diff_args(repo_root)
    branch_paths = collect_changed_paths(repo_root, branch_diff_args)
    staged_paths = collect_changed_paths(repo_root, ["--cached"])
    worktree_paths = collect_changed_paths(repo_root, [])
    untracked_paths = collect_untracked_paths(repo_root)

    labels = [branch_basis]
    if staged_paths:
        labels.append("index")
    if worktree_paths:
        labels.append("worktree")
    if untracked_paths:
        labels.append("untracked")

    changed = set(branch_paths)
    changed.update(staged_paths)
    changed.update(worktree_paths)
    changed.update(untracked_paths)
    return sorted(changed), " + ".join(labels)


def is_changed_source_file(repo_root: Path, path: str) -> bool:
    """Return whether a path should participate in changed-file coverage policy."""

    if not path.endswith(".rs"):
        return False
    if not path.startswith("src/"):
        return False
    if not (repo_root / path).exists():
        return False
    if path.startswith("src/") and "/src/" in path:
        return False
    if is_internal_test_module(path):
        return False
    return True


def changed_source_files(repo_root: Path, changed_paths: list[str]) -> list[str]:
    """Filter candidate paths down to production Rust source files under `src/`."""

    return sorted(
        {path for path in changed_paths if is_changed_source_file(repo_root, path)}
    )


def line_coverage(summary: dict[str, Any]) -> FileCoverage:
    """Normalize llvm-cov line summaries that may or may not include `percent`."""

    count = float(summary.get("count", 0))
    if "percent" in summary:
        percent = float(summary["percent"])
    else:
        covered = float(summary.get("covered", count))
        percent = 100.0 if count == 0 else (100.0 * covered / count)
    return FileCoverage(percent=percent, count=count)


def relative_report_path(filename: str, repo_root: Path) -> str:
    """Map llvm-cov filenames back into repo-relative paths when possible."""

    resolved = Path(filename).resolve()
    try:
        return resolved.relative_to(repo_root).as_posix()
    except ValueError:
        return os.path.normpath(filename)


def is_coverage_exempt_module(path: Path) -> bool:
    """Detect declaration-only modules that are poor per-file gate candidates.

    This is intentionally heuristic rather than a Rust parser. The goal is to
    avoid false failures for files that legitimately do not emit meaningful
    executable coverage entries, not to classify every Rust construct perfectly.
    """

    try:
        text = path.read_text()
        lines = text.splitlines()
    except OSError:
        return False

    # This early exit covers the common "types and re-exports only" case and
    # keeps the later line-by-line heuristic focused on edge cases.
    if "fn " not in text and "impl " not in text:
        return True

    allowed_prefixes = (
        "pub mod ",
        "pub(crate) mod ",
        "pub(super) mod ",
        "mod ",
        "pub use ",
        "pub(crate) use ",
        "pub(super) use ",
        "use ",
        "extern crate ",
        "pub type ",
        "pub(crate) type ",
        "pub(super) type ",
        "type ",
        "pub struct ",
        "pub(crate) struct ",
        "pub(super) struct ",
        "struct ",
        "pub enum ",
        "pub(crate) enum ",
        "pub(super) enum ",
        "enum ",
    )
    in_use_block = False

    for raw in lines:
        line = raw.strip()
        if not line:
            continue
        if line.startswith(("//", "//!", "///", "#!", "#[", "/*", "*", "*/")):
            continue
        if in_use_block:
            if line.endswith(";"):
                in_use_block = False
            continue
        if line in {"{", "}", "};"}:
            continue
        if line.startswith(allowed_prefixes):
            in_use_block = not line.endswith(";")
            continue
        return False

    return True


def choose_coverage_plan(repo_root: Path, *, fast: bool) -> CoveragePlan:
    """Choose the explicit coverage target set for this invocation.

    Fast mode is kept aligned with the non-PTY local confidence lane on purpose.
    Full mode is the authoritative release/CI gate.
    """

    changed_paths, diff_basis = collect_candidate_paths(repo_root)
    changed_files = changed_source_files(repo_root, changed_paths)

    if not fast:
        return CoveragePlan(
            mode="full",
            display_diff_basis=diff_basis,
            changed_files=changed_files,
            target_args=FULL_COVERAGE_TARGET_ARGS,
            target_label="lib/bins + unit + contracts + integration + e2e",
            reason="full gate runs the full instrumented test tier set",
        )

    if not changed_files:
        return CoveragePlan(
            mode="skip",
            display_diff_basis=diff_basis,
            changed_files=[],
            target_args=(),
            target_label="none",
            reason="no changed Rust source files across branch, index, or worktree",
        )

    return CoveragePlan(
        mode="fast",
        display_diff_basis=diff_basis,
        changed_files=changed_files,
        target_args=FAST_COVERAGE_TARGET_ARGS,
        target_label="lib/bins + contracts + integration",
        reason="fast gate checks changed-file coverage against the non-PTY local lane",
    )


def parse_report(report_path: Path, repo_root: Path) -> CoverageReport:
    """Parse the llvm-cov JSON export into the smaller shape the gate needs."""

    with report_path.open() as handle:
        report = json.load(handle)

    data = report["data"][0]
    overall = line_coverage(data["totals"]["lines"]).percent

    files: dict[str, FileCoverage] = {}
    for entry in data.get("files", []):
        rel = relative_report_path(entry["filename"], repo_root)
        lines = entry.get("summary", {}).get("lines", {})
        files[rel] = line_coverage(lines)

    return CoverageReport(overall=overall, files=files)


def normalize_llvm_cov_args(args: list[str]) -> list[str]:
    """Accept a passthrough `--` prefix without forcing callers to care."""

    if args and args[0] == "--":
        return args[1:]
    return args


def run_llvm_cov(args: list[str], *, repo_root: Path, prefix: str) -> None:
    """Run an arbitrary cargo-llvm-cov command inside the isolated workspace."""

    ensure_coverage_tooling(purpose="run cargo llvm-cov")
    llvm_cov_args = normalize_llvm_cov_args(args)
    with prepared_workspace(prefix) as (_, env):
        subprocess.run(
            ["cargo", "llvm-cov", *llvm_cov_args],
            cwd=repo_root,
            check=True,
            env=env,
        )


def build_json_report_command(
    report_path: Path, *, target_args: tuple[str, ...]
) -> list[str]:
    """Build the shared JSON export command for both full and fast gates."""

    command = [
        "cargo",
        "llvm-cov",
        *JSON_REPORT_BASE_ARGS,
        f"--output-path={report_path}",
    ]
    command.extend(target_args)
    return command


def run_json_report(
    repo_root: Path,
    *,
    prefix: str,
    target_args: tuple[str, ...],
) -> CoverageReport:
    """Execute llvm-cov and return the parsed JSON report."""

    with prepared_workspace(prefix) as (run_dir, env):
        report_path = run_dir / "coverage.json"
        if report_path.exists():
            report_path.unlink()
        subprocess.run(
            build_json_report_command(report_path, target_args=target_args),
            cwd=repo_root,
            check=True,
            env=env,
        )
        return parse_report(report_path, repo_root)


def run_coverage_plan(repo_root: Path, plan: CoveragePlan) -> CoverageReport | None:
    """Run the chosen coverage plan and echo the policy context first."""

    print(f"Coverage diff basis: {plan.display_diff_basis}", flush=True)
    print(f"Coverage mode: {plan.mode}", flush=True)
    if plan.mode != "skip":
        print(f"Coverage targets: {plan.target_label}", flush=True)
    print(f"Coverage reason: {plan.reason}", flush=True)

    if plan.mode == "skip":
        return None

    return run_json_report(
        repo_root,
        prefix="osp-cov-",
        target_args=plan.target_args,
    )


def evaluate_gate(
    repo_root: Path,
    *,
    plan: CoveragePlan,
    baseline: dict[str, float],
    report: CoverageReport | None,
    fast: bool,
) -> GateResult:
    """Apply repository coverage policy to the collected report.

    The overall baseline floor is reserved for full runs. Fast runs are there to
    catch obviously under-covered changed files without pretending to be as
    authoritative as the full CI gate.
    """

    errors: list[str] = []
    policy_notes: list[str] = []
    files_considered = len(plan.changed_files)
    files_checked = 0
    files_skipped_by_policy = 0

    overall = report.overall if report is not None else None
    files = report.files if report is not None else {}

    baseline_overall = baseline["overall_line_percent"]
    if overall is not None and not fast and overall + 1e-9 < baseline_overall:
        errors.append(
            f"overall line coverage regressed: baseline={baseline_overall:.2f}% current={overall:.2f}%"
        )

    min_file_percent = baseline["changed_file_min_line_percent"]
    min_executable_lines = baseline["min_executable_lines"]
    for path in plan.changed_files:
        entry = files.get(path)
        if entry is None:
            source_path = repo_root / path
            if is_coverage_exempt_module(source_path):
                policy_notes.append(
                    f"skipping declaration-only module coverage gate for {path}"
                )
                files_skipped_by_policy += 1
                continue
            errors.append(f"no coverage entry found for changed source file: {path}")
            continue
        if entry.count < min_executable_lines:
            policy_notes.append(
                f"skipping tiny file coverage gate for {path} ({entry.count:.0f} executable lines)"
            )
            files_skipped_by_policy += 1
            continue

        files_checked += 1
        if entry.percent + 1e-9 < min_file_percent:
            errors.append(
                f"changed file below {min_file_percent:.1f}%: {path} ({entry.percent:.2f}%)"
            )

    return GateResult(
        overall=overall,
        files_considered=files_considered,
        files_checked=files_checked,
        files_skipped_by_policy=files_skipped_by_policy,
        errors=errors,
        policy_notes=policy_notes,
    )


def render_gate_result(
    plan: CoveragePlan,
    result: GateResult,
    *,
    baseline: dict[str, float],
    fast: bool,
) -> None:
    """Render success or failure in terms of policy, not raw tool output."""

    min_file_percent = baseline["changed_file_min_line_percent"]
    if result.errors:
        print("\nCoverage gate failed:\n", file=sys.stderr)
        for error in result.errors:
            print(f"  - {error}", file=sys.stderr)
        if plan.changed_files:
            print(
                "\nChanged source files considered: "
                f"{result.files_considered}; checked against {min_file_percent:.1f}% minimum: "
                f"{result.files_checked}; skipped by policy: {result.files_skipped_by_policy}",
                file=sys.stderr,
            )
        if result.policy_notes:
            print("\nPolicy notes:", file=sys.stderr)
            for note in result.policy_notes:
                print(f"  - {note}", file=sys.stderr)
        raise SystemExit(1)

    baseline_overall = baseline["overall_line_percent"]
    if result.overall is not None and not fast:
        print(
            f"Coverage OK: overall {result.overall:.2f}% (baseline {baseline_overall:.2f}%)"
        )
    elif fast:
        print("Coverage OK: fast gate passed")
    else:
        print("Coverage OK")

    if plan.changed_files:
        print(
            "Changed source files considered: "
            f"{result.files_considered}; checked against {min_file_percent:.1f}% minimum: "
            f"{result.files_checked}; skipped by policy: {result.files_skipped_by_policy}"
        )
    elif fast and plan.mode == "skip":
        print("No changed source files in push range; fast gate skipped coverage.")
    else:
        print("No changed source files in push range; checked overall coverage only.")

    if result.policy_notes:
        print("Policy notes:")
        for note in result.policy_notes:
            print(f"  - {note}")


def command_run(args: argparse.Namespace) -> None:
    """Handle the passthrough `run` subcommand for manual llvm-cov use."""

    root = repo_root()
    run_llvm_cov(args.llvm_cov_args, repo_root=root, prefix="osp-cov-manual-")


def command_gate(args: argparse.Namespace) -> None:
    """Handle the repository coverage gate entry point."""

    root = repo_root()
    ensure_coverage_tooling(purpose="run the coverage gate")
    baseline_path = root / ".coverage-baseline.json"
    if not baseline_path.exists():
        fail(
            "Missing .coverage-baseline.json. Add a baseline before enabling the coverage gate."
        )

    baseline = load_baseline(baseline_path)
    plan = choose_coverage_plan(root, fast=args.fast)
    report = run_coverage_plan(root, plan)
    result = evaluate_gate(
        root,
        plan=plan,
        baseline=baseline,
        report=report,
        fast=args.fast,
    )
    render_gate_result(plan, result, baseline=baseline, fast=args.fast)


def command_baseline(_args: argparse.Namespace) -> None:
    """Recompute the full baseline when the team intentionally raises the floor."""

    root = repo_root()
    baseline_path = root / ".coverage-baseline.json"
    if not baseline_path.exists():
        fail("Missing .coverage-baseline.json.")

    ensure_coverage_tooling(purpose="update the coverage baseline")

    baseline_payload = load_baseline_payload(baseline_path)
    previous = float(baseline_payload["overall_line_percent"])

    print("Running full root-package coverage for baseline update...", flush=True)
    report = run_json_report(
        root,
        prefix="osp-cov-baseline-",
        target_args=FULL_COVERAGE_TARGET_ARGS,
    )
    current = report.overall

    baseline_payload["generated_at"] = date.today().isoformat()
    baseline_payload["overall_line_percent"] = round(current, 2)
    write_baseline_payload(baseline_path, baseline_payload)

    print(
        f"Updated baseline overall line coverage: {previous:.2f}% -> {current:.2f}%"
    )
    print("Review the change and commit it deliberately when you want to raise the floor.")


def build_parser() -> argparse.ArgumentParser:
    """Build the small CLI surface for coverage utility commands."""

    parser = argparse.ArgumentParser(
        description="Coverage utilities for osp-cli-rust."
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    run_parser = subparsers.add_parser(
        "run",
        usage="%(prog)s [llvm-cov args ...]",
        help="Run cargo llvm-cov inside the isolated coverage workspace.",
        description="Run cargo llvm-cov inside the isolated coverage workspace.",
        epilog="Additional arguments after `run` are passed through to cargo llvm-cov.",
    )
    run_parser.set_defaults(func=command_run)

    gate_parser = subparsers.add_parser(
        "gate",
        help="Run the repository coverage gate.",
    )
    gate_parser.add_argument(
        "--fast",
        action="store_true",
        help=(
            "Use a changed-file approximation instead of the full release-path coverage run. "
            "Intended for local pre-push use."
        ),
    )
    gate_parser.set_defaults(func=command_gate)

    baseline_parser = subparsers.add_parser(
        "baseline",
        help="Recompute and write the checked-in coverage baseline.",
    )
    baseline_parser.set_defaults(func=command_baseline)

    return parser


def parse_args(argv: list[str]) -> argparse.Namespace:
    """Parse arguments while preserving passthrough behavior for `run`."""

    parser = build_parser()
    args, extra = parser.parse_known_args(argv)
    if args.command == "run":
        # `run` is intentionally the one passthrough subcommand.
        args.llvm_cov_args = extra
        return args
    if extra:
        parser.error(f"unrecognized arguments: {' '.join(extra)}")
    return args


def main() -> None:
    """Parse CLI arguments and dispatch to the selected subcommand."""

    args = parse_args(sys.argv[1:])
    args.func(args)


if __name__ == "__main__":
    main()
