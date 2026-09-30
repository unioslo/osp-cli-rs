#!/usr/bin/env python3
"""Measure the already-built release CLI's warm startup and binary size."""

import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import runpy
import statistics
import subprocess
import tempfile
import time


WARMUPS = 3
SAMPLES = 11


def main() -> None:
    root = Path(__file__).resolve().parent.parent
    target = Path(os.environ.get("CARGO_TARGET_DIR", "target"))
    default_binary = root / target / os.environ.get("CARGO_BUILD_TARGET", "") / "release" / "osp"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=default_binary)
    parser.add_argument("--baseline", action="store_true", help="Record a deliberate release baseline.")
    args = parser.parse_args()
    binary = args.binary.resolve()
    if not binary.is_file():
        parser.error(f"release binary missing: {binary}; run confidence.py --check build first")
    baseline_path = root / ".release-budget.json"
    hermetic_env = runpy.run_path(str(root / "scripts/run-hermetic-cargo.py"))["hermetic_env"]
    durations = []
    with tempfile.TemporaryDirectory(prefix="osp-release-budget-") as home:
        env = hermetic_env(Path(home))
        for index in range(WARMUPS + SAMPLES):
            started = time.perf_counter()
            completed = subprocess.run(
                [str(binary), "--version"], cwd=root, env=env,
                capture_output=True, text=True, check=True, timeout=5,
            )
            if index >= WARMUPS:
                durations.append((time.perf_counter() - started) * 1000)
    startup_ms = statistics.median(durations)
    size_bytes = binary.stat().st_size
    print(f"Release startup: {startup_ms:.2f} ms median ({WARMUPS} warmups, {SAMPLES} samples); size: {size_bytes} bytes")
    if args.baseline:
        payload = {
            "startup_ms": startup_ms, "size_bytes": size_bytes,
            "profile": "release", "warmups": WARMUPS, "samples": SAMPLES,
            "measured_at": datetime.now(timezone.utc).isoformat(),
            "platform": platform.platform(), "version": completed.stdout.strip(),
            "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
            "source_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
            "source_dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=root)),
            "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        }
        baseline_path.write_text(json.dumps(payload, indent=2) + "\n")
        print(f"Recorded {baseline_path}; review deliberately before committing.")
        return
    baseline = json.loads(baseline_path.read_text())
    for key in ("startup_ms", "size_bytes"):
        value = baseline[key]
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value <= 0:
            parser.error(f"invalid release baseline {key}: {value}")
    startup_limit = max(1.5 * baseline["startup_ms"], 50)
    size_limit = 1.5 * baseline["size_bytes"]
    print(f"Budgets: startup <= {startup_limit:.2f} ms; size <= {size_limit:.0f} bytes")
    if startup_ms > startup_limit or size_bytes > size_limit:
        raise SystemExit("Release startup or binary size exceeds its budget.")


if __name__ == "__main__":
    main()
