#!/usr/bin/env python3
"""Measure clean, warm, offline, and cancelled compiler builds."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import platform
import shlex
import statistics
import subprocess
import tempfile
import time
from datetime import datetime, timezone


ROOT = pathlib.Path(__file__).resolve().parent.parent


def now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def run_build(command: list[str], target_dir: pathlib.Path, offline: bool) -> tuple[float, int]:
    started = time.monotonic_ns()
    measured_command = [*command, "--offline"] if offline else command
    environment = {**os.environ}
    if offline:
        environment["CARGO_NET_OFFLINE"] = "true"
    else:
        environment.pop("CARGO_NET_OFFLINE", None)
    result = subprocess.run(
        [*measured_command, "--target-dir", str(target_dir)],
        cwd=ROOT,
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )
    elapsed = (time.monotonic_ns() - started) / 1_000_000_000
    return elapsed, result.returncode


def run_cancelled(
    command: list[str], target_dir: pathlib.Path, delay: float, offline: bool
) -> tuple[float, int]:
    started = time.monotonic_ns()
    measured_command = [*command, "--offline"] if offline else command
    environment = {**os.environ}
    if offline:
        environment["CARGO_NET_OFFLINE"] = "true"
    else:
        environment.pop("CARGO_NET_OFFLINE", None)
    process = subprocess.Popen(
        [*measured_command, "--target-dir", str(target_dir)],
        cwd=ROOT,
        env=environment,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    time.sleep(delay)
    process.terminate()
    process.wait()
    elapsed = (time.monotonic_ns() - started) / 1_000_000_000
    return elapsed, process.returncode


def summary(values: list[float]) -> dict[str, float]:
    ordered = sorted(values)
    return {
        "min_seconds": min(ordered),
        "p50_seconds": statistics.median(ordered),
        "p95_seconds": ordered[min(len(ordered) - 1, int(len(ordered) * 0.95))],
        "max_seconds": max(ordered),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest-path", required=True)
    parser.add_argument("--bin", required=True)
    parser.add_argument("--target", default="x86_64")
    parser.add_argument("--release", action="store_true")
    parser.add_argument("--samples", type=int, default=5)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--cancel-after", type=float, default=0.25)
    parser.add_argument("--output", type=pathlib.Path)
    parser.add_argument(
        "--cargo-ghostos",
        default="cargo ghostos",
        help="command prefix for the cargo-ghostos subcommand",
    )
    args = parser.parse_args()
    if args.samples < 1 or args.warmups < 0 or args.cancel_after <= 0:
        parser.error("samples, warmups, and cancel-after must be positive")

    command = [
        *shlex.split(args.cargo_ghostos),
        "build",
        "--manifest-path",
        args.manifest_path,
        "--bin",
        args.bin,
        "--target",
        args.target,
        "--locked",
    ]
    if args.release:
        command.append("--release")

    measurements: dict[str, list[float]] = {"clean": [], "warm": [], "offline": [], "cancelled": []}
    with tempfile.TemporaryDirectory(prefix="ghostos-compiler-bench-") as temporary:
        root = pathlib.Path(temporary)
        warm_target = root / "warm"
        for _ in range(args.warmups):
            run_build(command, warm_target, False)
        for _ in range(args.samples):
            clean_target = root / f"clean-{len(measurements['clean'])}"
            elapsed, _ = run_build(command, clean_target, False)
            measurements["clean"].append(elapsed)
            elapsed, _ = run_build(command, warm_target, False)
            measurements["warm"].append(elapsed)
            elapsed, _ = run_build(command, warm_target, True)
            measurements["offline"].append(elapsed)
            cancelled_target = root / f"cancelled-{len(measurements['cancelled'])}"
            elapsed, _ = run_cancelled(command, cancelled_target, args.cancel_after, True)
            measurements["cancelled"].append(elapsed)

    report = {
        "schema": 1,
        "recorded_at": now(),
        "host": {
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
            "logical_cpus": os.cpu_count(),
        },
        "command": command,
        "samples": args.samples,
        "cancel_after_seconds": args.cancel_after,
        "workflows": {name: summary(values) for name, values in measurements.items()},
        "bound": "temporary workspaces; offline dependency resolution; cancellation has no published output",
    }
    output = args.output or ROOT / "build" / "benchmarks" / "compiler-workflows.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
