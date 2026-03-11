#!/usr/bin/env python3
from __future__ import annotations

import contextlib
import fcntl
import os
import shutil
import tempfile
from pathlib import Path


RUNS_TO_KEEP = 4


def coverage_root() -> Path:
    return Path(tempfile.gettempdir()) / "osp-cli-cov"


def coverage_runs_root() -> Path:
    return coverage_root() / "runs"


def coverage_tmp_root() -> Path:
    return coverage_root() / "tmp"


@contextlib.contextmanager
def coverage_lock() -> object:
    root = coverage_root()
    root.mkdir(parents=True, exist_ok=True)
    lock_path = root / ".lock"
    with lock_path.open("w") as handle:
        fcntl.flock(handle.fileno(), fcntl.LOCK_EX)
        yield


def clear_workspace_tmp() -> None:
    _clear_dir(coverage_tmp_root())


def _remove_path(path: Path) -> None:
    if path.is_dir() and not path.is_symlink():
        shutil.rmtree(path)
        return
    path.unlink(missing_ok=True)


def _clear_dir(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)
    for child in path.iterdir():
        _remove_path(child)


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
