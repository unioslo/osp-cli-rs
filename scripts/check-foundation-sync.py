#!/usr/bin/env python3
from __future__ import annotations

import filecmp
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
FOUNDATION_DIR = REPO_ROOT / "foundation"
GENERATOR = REPO_ROOT / "scripts" / "build-foundation-crate.py"
IGNORED_TOP_LEVEL = {"Cargo.lock", "target"}


def run(command: list[str], cwd: Path) -> None:
    subprocess.run(command, cwd=cwd, check=True)


def compare_dirs(expected: Path, actual: Path, *, rel: Path = Path(".")) -> list[str]:
    mismatches: list[str] = []
    comparison = filecmp.dircmp(expected, actual)

    for name in sorted(name for name in comparison.left_only if not ignored(rel, name)):
        mismatches.append(f"missing from generated foundation: {(rel / name).as_posix()}")
    for name in sorted(name for name in comparison.right_only if not ignored(rel, name)):
        mismatches.append(f"extra in generated foundation: {(rel / name).as_posix()}")
    for name in sorted(name for name in comparison.diff_files if not ignored(rel, name)):
        mismatches.append(f"content differs: {(rel / name).as_posix()}")
    for name in sorted(name for name in comparison.funny_files if not ignored(rel, name)):
        mismatches.append(f"uncomparable file: {(rel / name).as_posix()}")

    for name in sorted(name for name in comparison.common_dirs if not ignored(rel, name)):
        mismatches.extend(
            compare_dirs(expected / name, actual / name, rel=rel / name)
        )

    return mismatches


def ignored(rel: Path, name: str) -> bool:
    return rel == Path(".") and name in IGNORED_TOP_LEVEL


def main() -> int:
    if not FOUNDATION_DIR.exists():
        print("foundation/ does not exist; generate it first", file=sys.stderr)
        return 1

    with tempfile.TemporaryDirectory(prefix="osp-foundation-sync-") as temp_dir:
        generated = Path(temp_dir) / "foundation"
        run(
            [sys.executable, str(GENERATOR), "--out-dir", str(generated)],
            cwd=REPO_ROOT,
        )
        target_dir = generated / "target"
        if target_dir.exists():
            shutil.rmtree(target_dir)

        mismatches = compare_dirs(FOUNDATION_DIR, generated)
        if mismatches:
            print("foundation/ is out of sync with the workspace generator:", file=sys.stderr)
            for mismatch in mismatches:
                print(f"  - {mismatch}", file=sys.stderr)
            print(
                "\nRegenerate with: python3 scripts/build-foundation-crate.py --out-dir foundation",
                file=sys.stderr,
            )
            return 1

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
