#!/usr/bin/env python3
"""Run repeatable benchmarks and reject regressions without hiding noise."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import pathlib
import platform
import random
import shutil
import statistics
import subprocess
import sys
import time
import tomllib
from datetime import datetime, timezone


ROOT = pathlib.Path(__file__).resolve().parent.parent
DEFAULT_BUDGETS = ROOT / "benchmarks" / "budgets.toml"
DEFAULT_COMMAND = ["cargo", "bench", "-p", "synos-vm", "--bench", "bounded"]


def now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def git_revision() -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True, check=False
    )
    return result.stdout.strip() if result.returncode == 0 else "unknown"


def first_line(paths: list[pathlib.Path]) -> str | None:
    for path in paths:
        try:
            value = path.read_text().strip().splitlines()[0]
        except (OSError, IndexError):
            continue
        if value:
            return value
    return None


def command_output(command: list[str]) -> str | None:
    result = subprocess.run(command, capture_output=True, text=True, check=False)
    return result.stdout.strip() if result.returncode == 0 else None


def linux_cpu_model() -> str | None:
    try:
        for line in pathlib.Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith(("model name", "Hardware")):
                return line.split(":", 1)[1].strip()
    except (OSError, IndexError):
        return None
    return None


def physical_cpu_count() -> int | None:
    if platform.system() == "Darwin":
        value = command_output(["sysctl", "-n", "hw.physicalcpu"])
        return int(value) if value and value.isdigit() else None
    try:
        physical_ids = set()
        core_ids = set()
        physical_id = None
        core_id = None
        for line in pathlib.Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("physical id"):
                physical_id = line.split(":", 1)[1].strip()
            elif line.startswith("core id"):
                core_id = line.split(":", 1)[1].strip()
            elif not line.strip() and physical_id is not None and core_id is not None:
                physical_ids.add(physical_id)
                core_ids.add((physical_id, core_id))
                physical_id = None
                core_id = None
        if physical_ids and core_ids:
            return len(core_ids)
    except OSError:
        pass
    return None


def memory_bytes() -> int | None:
    if platform.system() == "Darwin":
        value = command_output(["sysctl", "-n", "hw.memsize"])
        return int(value) if value and value.isdigit() else None
    try:
        for line in pathlib.Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemTotal:"):
                return int(line.split()[1]) * 1024
    except (OSError, ValueError):
        pass
    try:
        return os.sysconf("SC_PHYS_PAGES") * os.sysconf("SC_PAGE_SIZE")
    except (AttributeError, OSError, ValueError):
        return None


def energy_paths() -> list[pathlib.Path]:
    root = pathlib.Path("/sys/class/powercap")
    if not root.is_dir():
        return []
    return sorted(root.glob("intel-rapl*/energy_uj"))


def energy_uj(paths: list[pathlib.Path]) -> int | None:
    if not paths:
        return None
    total = 0
    try:
        for path in paths:
            total += int(path.read_text().strip())
    except (OSError, ValueError):
        return None
    return total


def hardware_metadata() -> dict[str, object]:
    logical = os.cpu_count() or 1
    affinity = sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None
    cpu_model = first_line(
        [
            pathlib.Path("/sys/devices/virtual/dmi/id/product_name"),
            pathlib.Path("/sys/devices/virtual/dmi/id/board_name"),
        ]
    )
    if platform.system() == "Darwin":
        cpu_model = command_output(["sysctl", "-n", "machdep.cpu.brand_string"]) or cpu_model
    cpu_model = cpu_model or linux_cpu_model() or platform.processor()
    governor = first_line([pathlib.Path("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")])
    hardware = {
        "system": platform.system(),
        "kernel": platform.release(),
        "machine": platform.machine(),
        "cpu_model": cpu_model or "unknown",
        "logical_cpus": logical,
        "physical_cpus": physical_cpu_count(),
        "memory_bytes": memory_bytes(),
        "cpu_governor": governor,
        "allowed_cpus": affinity,
    }
    signature = hashlib.sha256(
        json.dumps(hardware, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()
    hardware["signature"] = signature
    return hardware


def quantile(values: list[float], probability: float) -> float:
    ordered = sorted(values)
    if len(ordered) == 1:
        return ordered[0]
    position = (len(ordered) - 1) * probability
    lower = math.floor(position)
    upper = math.ceil(position)
    if lower == upper:
        return ordered[lower]
    weight = position - lower
    return ordered[lower] + (ordered[upper] - ordered[lower]) * weight


def bootstrap_ci(
    values: list[float], probability: float, confidence: float, seed: int
) -> dict[str, float]:
    if len(values) < 2:
        value = values[0] if values else 0.0
        return {"low": value, "high": value}
    iterations = 512
    randomizer = random.Random(seed)
    estimates = []
    for _ in range(iterations):
        resample = [values[randomizer.randrange(len(values))] for _ in values]
        estimates.append(quantile(resample, probability))
    tail = (1.0 - confidence) / 2.0
    return {"low": quantile(estimates, tail), "high": quantile(estimates, 1.0 - tail)}


def metric_summary(values: list[float], confidence: float, seed: int) -> dict[str, object]:
    median = statistics.median(values)
    mean = statistics.fmean(values)
    mad = statistics.median([abs(value - median) for value in values])
    return {
        "count": len(values),
        "min": min(values),
        "max": max(values),
        "mean": mean,
        "median": median,
        "p50": quantile(values, 0.50),
        "p95": quantile(values, 0.95),
        "p99": quantile(values, 0.99),
        "mad": mad,
        "relative_mad": mad / abs(median) if median else 0.0,
        "confidence_interval": {
            "level": confidence,
            "mean": bootstrap_ci(values, 0.50, confidence, seed),
            "p50": bootstrap_ci(values, 0.50, confidence, seed + 1),
            "p95": bootstrap_ci(values, 0.95, confidence, seed + 2),
            "p99": bootstrap_ci(values, 0.99, confidence, seed + 3),
        },
    }


def parse_json_lines(stdout: str) -> tuple[dict[str, object] | None, list[dict[str, object]]]:
    metadata = None
    results = []
    for line in stdout.splitlines():
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(value, dict):
            continue
        if value.get("record") == "metadata":
            metadata = value
        elif value.get("record") == "result":
            results.append(value)
    return metadata, results


def parse_perf(stderr: str) -> int | None:
    for line in stderr.splitlines():
        fields = [field.strip() for field in line.split(",")]
        if len(fields) < 3 or fields[2] != "cycles":
            continue
        value = fields[0].replace(",", "")
        if value.isdigit():
            return int(value)
    return None


def run_once(
    command: list[str], revision: str, collect_optional: bool
) -> tuple[dict[str, object], int]:
    environment = os.environ.copy()
    environment["SYNOS_BENCH_REVISION"] = revision
    environment["SYNOS_BENCH_HARNESS"] = "1"
    paths = energy_paths()
    before_energy = energy_uj(paths)
    measured_command = command
    using_perf = False
    if collect_optional and platform.system() == "Linux" and shutil.which("perf"):
        measured_command = ["perf", "stat", "-x,", "-e", "cycles", "--", *command]
        using_perf = True
    started = time.monotonic_ns()
    result = subprocess.run(
        measured_command, cwd=ROOT, capture_output=True, text=True, env=environment, check=False
    )
    ended = time.monotonic_ns()
    if using_perf and result.returncode != 0 and any(
        marker in result.stderr.lower()
        for marker in ("permission", "not supported", "no permission", "cannot open")
    ):
        started = time.monotonic_ns()
        result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, env=environment, check=False)
        ended = time.monotonic_ns()
        using_perf = False
    after_energy = energy_uj(paths)
    metadata, benchmarks = parse_json_lines(result.stdout)
    sample = {
        "command": command,
        "returncode": result.returncode,
        "wall_elapsed_ns": ended - started,
        "cpu_cycles": parse_perf(result.stderr) if using_perf else None,
        "energy_uj": (
            after_energy - before_energy
            if before_energy is not None and after_energy is not None and after_energy >= before_energy
            else None
        ),
        "metadata": metadata,
        "benchmarks": benchmarks,
    }
    if result.returncode != 0:
        sample["stderr"] = result.stderr[-4000:]
        sample["stdout"] = result.stdout[-4000:]
    return sample, result.returncode


def load_config(path: pathlib.Path) -> tuple[dict[str, object], dict[str, dict[str, object]]]:
    document = tomllib.loads(path.read_text())
    defaults = document.get("defaults", {})
    benchmark = document.get("benchmark", {})
    if not isinstance(defaults, dict) or not isinstance(benchmark, dict):
        raise ValueError("benchmark budgets need [defaults] and [benchmark.<name>] tables")
    return defaults, {str(name): value for name, value in benchmark.items() if isinstance(value, dict)}


def budget_for(
    defaults: dict[str, object], benchmarks: dict[str, dict[str, object]], name: str
) -> dict[str, object]:
    value = dict(defaults)
    value.update(benchmarks.get(name, {}))
    return value


def aggregate(
    samples: list[dict[str, object]], confidence: float, names: list[str]
) -> dict[str, dict[str, object]]:
    reports = {}
    for name in names:
        records = [
            record
            for sample in samples
            for record in sample["benchmarks"]
            if record.get("benchmark") == name
        ]
        first = records[0]
        values = {
            "latency_ns": [float(record["elapsed_ns"]) for record in records],
            "throughput_per_second": [
                float(record.get("rate_per_second", 0)) for record in records
            ],
            "allocation_count": [float(record["allocation_count"]) for record in records],
            "allocated_bytes": [float(record["allocated_bytes"]) for record in records],
            "peak_live_bytes": [float(record["peak_live_bytes"]) for record in records],
        }
        reports[name] = {
            "unit": first.get("unit"),
            "work_units": first.get("work_units"),
            "checksum": first.get("checksum"),
            "latency_ns": metric_summary(values["latency_ns"], confidence, 100),
            "throughput_per_second": metric_summary(values["throughput_per_second"], confidence, 200),
            "allocation_count": metric_summary(values["allocation_count"], confidence, 300),
            "allocated_bytes": metric_summary(values["allocated_bytes"], confidence, 400),
            "peak_live_bytes": metric_summary(values["peak_live_bytes"], confidence, 500),
        }
    return reports


def optional_run_metrics(
    samples: list[dict[str, object]], confidence: float
) -> dict[str, dict[str, object]]:
    metrics = {
        "wall_elapsed_ns": [float(sample["wall_elapsed_ns"]) for sample in samples],
        "cpu_cycles": [
            float(sample["cpu_cycles"])
            for sample in samples
            if sample.get("cpu_cycles") is not None
        ],
        "energy_uj": [
            float(sample["energy_uj"])
            for sample in samples
            if sample.get("energy_uj") is not None
        ],
    }
    return {
        name: metric_summary(values, confidence, 600 + index)
        for index, (name, values) in enumerate(metrics.items())
        if values
    }


def compare_baseline(
    report: dict[str, object], baseline_path: pathlib.Path | None, defaults: dict[str, object], budgets: dict[str, dict[str, object]]
) -> tuple[list[str], list[str]]:
    failures = []
    inconclusive = []
    if baseline_path is None:
        return failures, inconclusive
    try:
        baseline = json.loads(baseline_path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        return failures, [f"cannot read baseline: {error}"]
    if baseline.get("hardware", {}).get("signature") != report["hardware"].get("signature"):
        return failures, ["baseline hardware signature differs"]
    for name, current in report["benchmarks"].items():
        previous = baseline.get("benchmarks", {}).get(name)
        if not isinstance(previous, dict):
            inconclusive.append(f"baseline has no benchmark {name}")
            continue
        budget = budget_for(defaults, budgets, name)
        latency_limit = float(budget.get("max_relative_latency_regression", 0.15))
        throughput_floor = float(budget.get("min_relative_throughput", 0.85))
        allocation_limit = float(budget.get("max_relative_allocation_regression", 0.25))
        current_latency = float(current["latency_ns"]["p50"])
        previous_latency = float(previous["latency_ns"]["p50"])
        if current_latency > previous_latency * (1.0 + latency_limit):
            failures.append(
                f"{name} p50 latency {current_latency:.0f}ns exceeds baseline budget "
                f"{previous_latency * (1.0 + latency_limit):.0f}ns"
            )
        current_throughput = float(current["throughput_per_second"]["p50"])
        previous_throughput = float(previous["throughput_per_second"]["p50"])
        if current_throughput < previous_throughput * throughput_floor:
            failures.append(
                f"{name} p50 throughput {current_throughput:.2f}/s is below baseline budget "
                f"{previous_throughput * throughput_floor:.2f}/s"
            )
        current_allocations = float(current["allocation_count"]["p50"])
        previous_allocations = float(previous["allocation_count"]["p50"])
        if current_allocations > previous_allocations * (1.0 + allocation_limit):
            failures.append(
                f"{name} p50 allocations {current_allocations:.0f} exceeds baseline budget "
                f"{previous_allocations * (1.0 + allocation_limit):.0f}"
            )
    return failures, inconclusive


def absolute_budget_checks(
    report: dict[str, object], defaults: dict[str, object], budgets: dict[str, dict[str, object]]
) -> list[str]:
    failures = []
    for name, current in report["benchmarks"].items():
        budget = budget_for(defaults, budgets, name)
        max_latency = budget.get("max_latency_ns")
        if max_latency is not None and float(current["latency_ns"]["p99"]) > float(max_latency):
            failures.append(f"{name} p99 latency exceeds declared max_latency_ns")
        min_throughput = budget.get("min_throughput_per_second")
        if min_throughput is not None and float(current["throughput_per_second"]["p50"]) < float(min_throughput):
            failures.append(f"{name} p50 throughput is below declared min_throughput_per_second")
        max_allocations = budget.get("max_allocation_count")
        if max_allocations is not None and float(current["allocation_count"]["p50"]) > float(max_allocations):
            failures.append(f"{name} p50 allocations exceed declared max_allocation_count")
    return failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--budgets", type=pathlib.Path, default=DEFAULT_BUDGETS)
    parser.add_argument("--baseline", type=pathlib.Path)
    parser.add_argument("--write-baseline", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path)
    parser.add_argument("--samples", type=int)
    parser.add_argument("--warmups", type=int)
    parser.add_argument("--pin-cpu", type=int)
    parser.add_argument("--no-optional-metrics", action="store_true")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    defaults, budgets = load_config(args.budgets)
    samples_count = args.samples or int(defaults.get("samples", 9))
    warmups_count = args.warmups if args.warmups is not None else int(defaults.get("warmups", 2))
    confidence = float(defaults.get("confidence", 0.95))
    if samples_count < 2 or warmups_count < 0:
        parser.error("samples must be at least 2 and warmups cannot be negative")
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    command = command or DEFAULT_COMMAND
    if args.pin_cpu is not None:
        if not hasattr(os, "sched_setaffinity"):
            parser.error("--pin-cpu needs an operating system with sched_setaffinity")
        os.sched_setaffinity(0, {args.pin_cpu})

    revision = git_revision()
    started_at = now()
    samples = []
    errors = []
    for index in range(warmups_count + samples_count):
        sample, returncode = run_once(command, revision, not args.no_optional_metrics)
        if returncode != 0:
            errors.append(f"benchmark command failed at run {index + 1}")
            break
        if index >= warmups_count:
            sample["sample"] = index - warmups_count + 1
            samples.append(sample)

    names = []
    if samples:
        if any(sample.get("metadata") is None for sample in samples):
            errors.append("benchmark metadata record is missing")
        if any(not sample["benchmarks"] for sample in samples):
            errors.append("benchmark result records are missing")
        names = [str(record["benchmark"]) for record in samples[0]["benchmarks"]]
        reference_metadata = samples[0].get("metadata")
        for sample in samples[1:]:
            if sample.get("metadata") != reference_metadata:
                errors.append("benchmark metadata changed between samples")
                break
    for sample in samples:
        found = [str(record.get("benchmark")) for record in sample["benchmarks"]]
        if found != names:
            errors.append("benchmark result set changed between samples")
            break
        for record in sample["benchmarks"]:
            reference = samples[0]["benchmarks"][names.index(record["benchmark"])]
            if (
                record.get("checksum") != reference.get("checksum")
                or record.get("unit") != reference.get("unit")
                or record.get("work_units") != reference.get("work_units")
            ):
                errors.append(f"{record.get('benchmark')} workload definition changed between samples")
                break

    hardware = hardware_metadata()
    report: dict[str, object] = {
        "schema": 1,
        "suite": "synos-vm-bounded",
        "status": "failed" if errors else "passed",
        "revision": revision,
        "started_at": started_at,
        "ended_at": now(),
        "command": command,
        "configuration": {
            "warmups": warmups_count,
            "samples": samples_count,
            "confidence": confidence,
            "optional_metrics": not args.no_optional_metrics,
        },
        "hardware": hardware,
        "samples": samples,
        "run_metrics": optional_run_metrics(samples, confidence) if samples else {},
        "benchmarks": aggregate(samples, confidence, names) if samples and not errors else {},
        "errors": errors,
        "regressions": [],
        "inconclusive_reasons": [],
        "regression_check": "checked" if args.baseline else "not-run",
    }

    if not errors:
        max_mad = float(defaults.get("max_relative_mad", 0.15))
        max_ci_width = float(defaults.get("max_latency_ci_width", 0.25))
        for name, current in report["benchmarks"].items():
            latency = current["latency_ns"]
            if float(latency["relative_mad"]) > max_mad:
                report["inconclusive_reasons"].append(
                    f"{name} latency relative MAD is {latency['relative_mad']:.3f}, above {max_mad:.3f}"
                )
            interval = latency["confidence_interval"]["mean"]
            width = (float(interval["high"]) - float(interval["low"])) / float(latency["mean"])
            if width > max_ci_width:
                report["inconclusive_reasons"].append(
                    f"{name} latency confidence interval width is {width:.3f}, above {max_ci_width:.3f}"
                )
        regression_failures, regression_inconclusive = compare_baseline(
            report, args.baseline, defaults, budgets
        )
        report["regressions"] = regression_failures
        report["inconclusive_reasons"].extend(regression_inconclusive)
        report["regressions"].extend(absolute_budget_checks(report, defaults, budgets))
        if report["regressions"]:
            report["status"] = "failed"
        elif report["inconclusive_reasons"]:
            report["status"] = "inconclusive"

    output = args.output
    if output is None:
        output = ROOT / "build" / "benchmarks" / f"{datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ')}-{os.getpid()}" / "report.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    if args.write_baseline:
        if report["status"] != "passed":
            print("refusing to write a non-passing baseline", file=sys.stderr)
            return 1
        args.write_baseline.parent.mkdir(parents=True, exist_ok=True)
        args.write_baseline.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps({"status": report["status"], "report": str(output)}))
    if report["status"] == "failed":
        return 1
    if report["status"] == "inconclusive":
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
