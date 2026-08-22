#!/usr/bin/env python3
"""Build and validate the evidence-backed GhostOS release report."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import math
import pathlib
import subprocess
import sys
import tomllib
from typing import Iterable


ROOT = pathlib.Path(__file__).resolve().parent.parent
BENCHMARK_BUDGETS = ROOT / "benchmarks/budgets.toml"
MAX_EVIDENCE_RESULTS = 4096
MAX_LATENCY_METRICS = 1024
MAX_RECOVERY_SAMPLES = 1024
MAX_KNOWN_LIMITS = 64
DEFAULT_SCALE_TIERS = (1, 2, 8, 32, 128)
DEFAULT_KNOWN_LIMITS = (
    "The portable CPU executor is the correctness path; native acceleration does not replace guest execution.",
    "KVM requires Linux and /dev/kvm; HVF requires macOS; WHPX requires Windows; HAXM requires its host device and supported platform.",
    "Migration authentication does not encrypt TCP; use mutually authenticated TLS, an authorized VPN, or an SSH tunnel.",
    "Raw terminal mode is host-owned and unavailable on non-TTY streams.",
    "The default network backend is an in-process loopback hub; external connectivity is not part of the VM artifact.",
    "QEMU, hardware, fuzz, coverage, mutation, cluster, and soak evidence may be skipped when host prerequisites are absent.",
)


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def git_revision() -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True, check=False
    )
    if result.returncode != 0 or not result.stdout.strip():
        raise ValueError("cannot determine current Git revision")
    return result.stdout.strip()


def vm_version() -> str:
    manifest = ROOT / "virtual_machine/Cargo.toml"
    with manifest.open("rb") as stream:
        package = tomllib.load(stream).get("package", {})
    return str(package.get("version", "unknown"))


def read_json(path: pathlib.Path) -> dict[str, object]:
    try:
        value = json.loads(path.read_text())
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"{path}: invalid JSON ({error})") from error
    if not isinstance(value, dict):
        raise ValueError(f"{path}: JSON root must be an object")
    return value


def numeric(value: object, field: str, *, integer: bool = False) -> int | float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{field} must be numeric")
    if not math.isfinite(float(value)) or value < 0:
        raise ValueError(f"{field} must be finite and non-negative")
    if integer and not isinstance(value, int):
        raise ValueError(f"{field} must be an integer")
    return value


def relative_path(path: pathlib.Path, root: pathlib.Path) -> str:
    try:
        return path.resolve().relative_to(root.resolve()).as_posix()
    except ValueError:
        return path.resolve().as_posix()


def evidence_records(evidence_dir: pathlib.Path) -> list[dict[str, object]]:
    candidates = sorted(
        path
        for path in evidence_dir.rglob("*")
        if path.is_file() and not path.is_symlink() and path.name in {"result.json", "evidence.json"}
    )
    records: list[dict[str, object]] = []
    for path in candidates:
        value = read_json(path)
        state = value.get("result_state", value.get("state"))
        if state == "pass":
            state = "passed"
        if state == "fail":
            state = "failed"
        if state not in {"passed", "failed", "skipped", "inconclusive"}:
            continue
        reason = value.get("reason") or value.get("prerequisite")
        if not isinstance(reason, str) or not reason.strip():
            raise ValueError(f"{path}: correctness result has no reason")
        records.append(
            {
                "path": relative_path(path, evidence_dir),
                "sha256": sha256(path),
                "state": state,
                "tier": value.get("tier"),
                "test_id": value.get("test_id"),
                "command": value.get("command"),
                "reason": reason.strip(),
            }
        )
        if len(records) > MAX_EVIDENCE_RESULTS:
            raise ValueError(f"evidence contains more than {MAX_EVIDENCE_RESULTS} correctness records")
    if not records:
        raise ValueError(f"{evidence_dir}: no correctness result records")
    return records


def source_entry(path: pathlib.Path, root: pathlib.Path) -> dict[str, str]:
    try:
        name = path.resolve().relative_to(root.resolve()).as_posix()
    except ValueError:
        name = path.name
    return {"path": name, "sha256": sha256(path)}


def report_paths(
    evidence_dir: pathlib.Path, explicit: Iterable[pathlib.Path], names: set[str]
) -> list[pathlib.Path]:
    paths = {path.expanduser().resolve() for path in explicit}
    paths.update(path.resolve() for path in evidence_dir.rglob("*") if path.is_file() and path.name in names)
    return sorted(paths)


def check_revision(report: dict[str, object], path: pathlib.Path, revision: str) -> None:
    if report.get("revision") != revision:
        raise ValueError(f"{path}: revision does not match HEAD")


def latency_metrics(paths: list[pathlib.Path], revision: str) -> tuple[list[dict[str, object]], list[pathlib.Path]]:
    metrics: list[dict[str, object]] = []
    sources: list[pathlib.Path] = []
    for path in paths:
        report = read_json(path)
        check_revision(report, path, revision)
        if report.get("status") != "passed":
            raise ValueError(f"{path}: benchmark report is not passed")
        benchmarks = report.get("benchmarks")
        if not isinstance(benchmarks, dict):
            raise ValueError(f"{path}: benchmark report has no benchmarks")
        for name, benchmark in benchmarks.items():
            if not isinstance(name, str) or not isinstance(benchmark, dict):
                raise ValueError(f"{path}: malformed benchmark entry")
            latency = benchmark.get("latency_ns")
            if not isinstance(latency, dict):
                raise ValueError(f"{path}: {name} has no latency summary")
            values = {percentile: numeric(latency.get(percentile), f"{path}: {name} {percentile}") for percentile in ("p50", "p95", "p99")}
            metrics.append(
                {
                    "source": path.name,
                    "benchmark": name,
                    "unit": benchmark.get("unit"),
                    "latency_ns": values,
                }
            )
            if len(metrics) > MAX_LATENCY_METRICS:
                raise ValueError(f"release report has more than {MAX_LATENCY_METRICS} latency metrics")
        sources.append(path)
    if not metrics:
        raise ValueError("no benchmark report with p50/p95/p99 latency data was supplied")
    return metrics, sources


def recovery_samples(paths: list[pathlib.Path], revision: str) -> tuple[list[dict[str, object]], list[pathlib.Path]]:
    samples: list[dict[str, object]] = []
    sources: list[pathlib.Path] = []
    for path in paths:
        report = read_json(path)
        check_revision(report, path, revision)
        raw_samples = report.get("samples", report.get("recoveries", report.get("results")))
        if not isinstance(raw_samples, list):
            raise ValueError(f"{path}: recovery report needs samples")
        for index, sample in enumerate(raw_samples):
            if not isinstance(sample, dict):
                raise ValueError(f"{path}: recovery sample {index} is not an object")
            fault = sample.get("fault")
            workflow = sample.get("workflow")
            if not isinstance(fault, str) or not fault.strip() or not isinstance(workflow, str) or not workflow.strip():
                raise ValueError(f"{path}: recovery sample {index} needs fault and workflow")
            observed = sample.get("observed_rto_ms", sample.get("recovery_ms", sample.get("duration_ms")))
            budget = sample.get("rto_budget_ms", sample.get("budget_ms"))
            observed_value = numeric(observed, f"{path}: recovery sample {index} observed_rto_ms", integer=True)
            budget_value = numeric(budget, f"{path}: recovery sample {index} rto_budget_ms", integer=True)
            state = sample.get("state", "passed")
            if state != "passed":
                raise ValueError(f"{path}: recovery sample {index} is not passed")
            if observed_value > budget_value:
                raise ValueError(f"{path}: recovery sample {index} exceeds its RTO budget")
            samples.append(
                {
                    "fault": fault.strip(),
                    "workflow": workflow.strip(),
                    "observed_rto_ms": observed_value,
                    "rto_budget_ms": budget_value,
                    "state": state,
                    "source": path.name,
                }
            )
            if len(samples) > MAX_RECOVERY_SAMPLES:
                raise ValueError(f"release report has more than {MAX_RECOVERY_SAMPLES} recovery samples")
        sources.append(path)
    if not samples:
        raise ValueError("no fault recovery samples were supplied")
    return samples, sources


def percentile(values: list[int], probability: float) -> int:
    ordered = sorted(values)
    index = min(len(ordered) - 1, math.ceil((len(ordered) - 1) * probability))
    return ordered[index]


def recovery_summary(samples: list[dict[str, object]]) -> dict[str, int]:
    values = [int(sample["observed_rto_ms"]) for sample in samples]
    return {
        "count": len(values),
        "p50_ms": percentile(values, 0.50),
        "p95_ms": percentile(values, 0.95),
        "p99_ms": percentile(values, 0.99),
        "max_ms": max(values),
    }


def resource_ceilings(soak_paths: list[pathlib.Path], extra: list[str]) -> tuple[list[dict[str, object]], list[pathlib.Path]]:
    try:
        config = tomllib.loads(BENCHMARK_BUDGETS.read_text())
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise ValueError(f"cannot read benchmark budgets: {error}") from error
    ceilings: list[dict[str, object]] = []
    for name, budget in sorted(config.get("benchmark", {}).items()):
        if not isinstance(budget, dict):
            continue
        if "max_latency_ns" in budget:
            ceilings.append({"name": f"benchmark.{name}.max_latency", "value": budget["max_latency_ns"], "unit": "ns", "source": "benchmarks/budgets.toml"})
        if "max_allocation_count" in budget:
            ceilings.append({"name": f"benchmark.{name}.max_allocations", "value": budget["max_allocation_count"], "unit": "allocations", "source": "benchmarks/budgets.toml"})
    for path in soak_paths:
        report = read_json(path)
        for name, value, unit in (
            ("soak.timeout", report.get("timeout_seconds"), "seconds"),
            ("soak.memory_tolerance", report.get("memory_tolerance_bytes"), "bytes"),
        ):
            if value is not None:
                ceilings.append({"name": name, "value": numeric(value, f"{path}: {name}"), "unit": unit, "source": path.name})
        declared = report.get("resource_ceilings")
        if isinstance(declared, dict):
            for name, value in declared.items():
                ceilings.append({"name": str(name), "value": numeric(value, f"{path}: resource ceiling {name}"), "unit": "count", "source": path.name})
    for raw in extra:
        if "=" not in raw:
            raise ValueError(f"resource ceiling must use NAME=VALUE[:UNIT]: {raw}")
        name, value_unit = raw.split("=", 1)
        value_text, separator, unit = value_unit.partition(":")
        if not name.strip() or not value_text.strip():
            raise ValueError(f"resource ceiling must use NAME=VALUE[:UNIT]: {raw}")
        try:
            value: int | float = int(value_text)
        except ValueError:
            try:
                value = float(value_text)
            except ValueError as error:
                raise ValueError(f"resource ceiling value is not numeric: {raw}") from error
        ceilings.append({"name": name.strip(), "value": numeric(value, raw), "unit": unit if separator and unit else "count", "source": "operator"})
    if not ceilings:
        raise ValueError("release report has no resource ceilings")
    return ceilings, soak_paths


def scale_report(benchmark_paths: list[pathlib.Path], explicit: list[int]) -> dict[str, object]:
    tiers = set(explicit)
    source = "docs/scalability.md"
    for path in benchmark_paths:
        report = read_json(path)
        for sample in report.get("samples", []):
            metadata = sample.get("metadata") if isinstance(sample, dict) else None
            if isinstance(metadata, dict) and isinstance(metadata.get("tiers"), list):
                tiers.update(int(value) for value in metadata["tiers"] if isinstance(value, int) and value > 0)
                source = path.name
    if not tiers:
        tiers.update(DEFAULT_SCALE_TIERS)
    ordered = sorted(tiers)
    return {
        "tiers": ordered,
        "max_cpu_count": ordered[-1],
        "basis": "declared policy and available scale benchmark metadata",
        "source": source,
    }


def validate_report(report_path: pathlib.Path, evidence_dir: pathlib.Path | None = None) -> dict[str, object]:
    report = read_json(report_path)
    if report.get("schema") != 1 or report.get("kind") != "ghostos-release-report":
        raise ValueError("release report has an unsupported schema")
    revision = git_revision()
    if report.get("revision") != revision:
        raise ValueError("release report revision does not match HEAD")
    if report.get("state") != "passed":
        raise ValueError("release report is not passed")
    correctness = report.get("correctness")
    if not isinstance(correctness, dict) or correctness.get("state") != "passed":
        raise ValueError("release report has no passed correctness result")
    results = correctness.get("results")
    if not isinstance(results, list) or not results:
        raise ValueError("release report has no correctness results")
    counts = correctness.get("counts")
    if not isinstance(counts, dict):
        raise ValueError("release report has no correctness counts")
    actual_counts = {state: 0 for state in ("passed", "failed", "skipped", "inconclusive")}
    if evidence_dir is not None:
        for result in results:
            if not isinstance(result, dict):
                raise ValueError("release report has a malformed correctness result")
            state = result.get("state")
            if state not in actual_counts:
                raise ValueError("release report has an invalid correctness state")
            actual_counts[state] += 1
            relative = pathlib.PurePosixPath(str(result.get("path")))
            if relative.is_absolute() or ".." in relative.parts:
                raise ValueError("release report correctness path escapes evidence directory")
            candidate = evidence_dir / pathlib.Path(*relative.parts)
            if not candidate.is_file() or sha256(candidate) != result.get("sha256"):
                raise ValueError(f"release report correctness source is missing or changed: {result.get('path')}")
    else:
        for result in results:
            if not isinstance(result, dict) or result.get("state") not in actual_counts:
                raise ValueError("release report has an invalid correctness state")
            actual_counts[result["state"]] += 1
    if counts != actual_counts:
        raise ValueError("release report correctness counts are stale")
    latency = report.get("latency")
    metrics = latency.get("metrics") if isinstance(latency, dict) else None
    if not isinstance(metrics, list) or not metrics:
        raise ValueError("release report has no latency metrics")
    for metric in metrics:
        values = metric.get("latency_ns") if isinstance(metric, dict) else None
        if not isinstance(values, dict):
            raise ValueError("release report latency metric has no p50/p95/p99 data")
        for percentile_name in ("p50", "p95", "p99"):
            numeric(values.get(percentile_name), f"latency {percentile_name}")
    if len(metrics) > MAX_LATENCY_METRICS:
        raise ValueError("release report has too many latency metrics")
    ceilings = report.get("resource_ceilings")
    if not isinstance(ceilings, list) or not ceilings:
        raise ValueError("release report has no resource ceilings")
    for ceiling in ceilings:
        if not isinstance(ceiling, dict) or not isinstance(ceiling.get("name"), str) or not isinstance(ceiling.get("unit"), str):
            raise ValueError("release report has a malformed resource ceiling")
        numeric(ceiling.get("value"), f"resource ceiling {ceiling.get('name')}")
    recovery = report.get("fault_recovery")
    if not isinstance(recovery, dict) or not isinstance(recovery.get("samples"), list) or not recovery["samples"]:
        raise ValueError("release report has no fault recovery times")
    if len(recovery["samples"]) > MAX_RECOVERY_SAMPLES:
        raise ValueError("release report has too many fault recovery samples")
    for sample in recovery["samples"]:
        if not isinstance(sample, dict) or sample.get("state") != "passed":
            raise ValueError("release report has a failed fault recovery sample")
        observed = numeric(sample.get("observed_rto_ms"), "fault recovery observed_rto_ms", integer=True)
        budget = numeric(sample.get("rto_budget_ms"), "fault recovery rto_budget_ms", integer=True)
        if observed > budget:
            raise ValueError("release report has a fault recovery sample over budget")
    summary = recovery.get("summary")
    if not isinstance(summary, dict) or any(name not in summary for name in ("p50_ms", "p95_ms", "p99_ms")):
        raise ValueError("release report has no fault recovery percentile summary")
    if summary != recovery_summary(recovery["samples"]):
        raise ValueError("release report fault recovery summary is stale")
    scale = report.get("supported_scale_tier")
    if not isinstance(scale, dict) or not isinstance(scale.get("tiers"), list) or not scale["tiers"]:
        raise ValueError("release report has no supported scale tier")
    tiers = scale["tiers"]
    if any(isinstance(tier, bool) or not isinstance(tier, int) or tier < 1 for tier in tiers) or scale.get("max_cpu_count") != max(tiers):
        raise ValueError("release report has an invalid supported scale tier")
    limits = report.get("known_limits")
    if not isinstance(limits, list) or not limits or not all(isinstance(limit, str) and limit.strip() for limit in limits):
        raise ValueError("release report has no known limits")
    if len(limits) > MAX_KNOWN_LIMITS:
        raise ValueError("release report has too many known limits")
    sources = report.get("sources")
    budget_source = next(
        (source for source in sources if isinstance(source, dict) and source.get("path") == "benchmarks/budgets.toml"),
        None,
    ) if isinstance(sources, list) else None
    if not isinstance(budget_source, dict) or budget_source.get("sha256") != sha256(BENCHMARK_BUDGETS):
        raise ValueError("release report benchmark budget source is missing or changed")
    return report


def build_report(args: argparse.Namespace) -> dict[str, object]:
    evidence_dir = args.evidence_dir.expanduser().resolve()
    if not evidence_dir.is_dir():
        raise ValueError(f"evidence directory does not exist: {evidence_dir}")
    revision = git_revision()
    correctness_results = evidence_records(evidence_dir)
    benchmark_paths = report_paths(evidence_dir, args.benchmark_report, {"benchmark-report.json"})
    metrics, benchmark_sources = latency_metrics(benchmark_paths, revision)
    recovery_paths = report_paths(evidence_dir, args.recovery_report, {"fault-recovery-report.json", "recovery-report.json"})
    recovery, recovery_sources = recovery_samples(recovery_paths, revision)
    soak_paths = report_paths(evidence_dir, [], {"report.json"})
    soak_paths = [path for path in soak_paths if "soak" in path.parts]
    ceilings, soak_sources = resource_ceilings(soak_paths, args.resource_ceiling)
    counts = {state: sum(record["state"] == state for record in correctness_results) for state in ("passed", "failed", "skipped", "inconclusive")}
    correctness_state = "passed" if counts["failed"] == 0 and counts["inconclusive"] == 0 else "failed"
    report: dict[str, object] = {
        "schema": 1,
        "kind": "ghostos-release-report",
        "state": "passed" if correctness_state == "passed" else "failed",
        "product": "ghostos-vm",
        "version": vm_version(),
        "revision": revision,
        "generated_at": dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
        "correctness": {"state": correctness_state, "counts": counts, "results": correctness_results},
        "latency": {"metrics": metrics, "percentiles": ["p50", "p95", "p99"]},
        "resource_ceilings": ceilings,
        "fault_recovery": {"samples": recovery, "summary": recovery_summary(recovery), "units": "milliseconds"},
        "supported_scale_tier": scale_report(benchmark_paths, args.scale_tier),
        "known_limits": list(dict.fromkeys((*DEFAULT_KNOWN_LIMITS, *args.known_limit)))[:MAX_KNOWN_LIMITS],
        "report_limits": {
            "max_correctness_records": MAX_EVIDENCE_RESULTS,
            "max_latency_metrics": MAX_LATENCY_METRICS,
            "max_recovery_samples": MAX_RECOVERY_SAMPLES,
            "max_known_limits": MAX_KNOWN_LIMITS,
        },
        "sources": [
            *[source_entry(path, evidence_dir) for path in benchmark_sources + recovery_sources + soak_sources],
            {"path": "benchmarks/budgets.toml", "sha256": sha256(BENCHMARK_BUDGETS)},
        ],
    }
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--write", action="store_true")
    mode.add_argument("--check", action="store_true")
    parser.add_argument("--evidence-dir", required=True, type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path)
    parser.add_argument("--report", type=pathlib.Path)
    parser.add_argument("--benchmark-report", action="append", default=[], type=pathlib.Path)
    parser.add_argument("--recovery-report", action="append", default=[], type=pathlib.Path)
    parser.add_argument("--resource-ceiling", action="append", default=[])
    parser.add_argument("--scale-tier", action="append", default=[], type=int)
    parser.add_argument("--known-limit", action="append", default=[])
    args = parser.parse_args()
    if args.write and args.output is None:
        parser.error("--write requires --output")
    if args.check and args.report is None:
        parser.error("--check requires --report")
    if any(value < 1 for value in args.scale_tier):
        parser.error("--scale-tier values must be positive")
    try:
        if args.check:
            validate_report(args.report.expanduser().resolve(), args.evidence_dir.expanduser().resolve())
            print(f"release report verified: {args.report.expanduser().resolve()}")
        else:
            report = build_report(args)
            output = args.output.expanduser().resolve()
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
            validate_report(output, args.evidence_dir.expanduser().resolve())
            print(f"wrote {output}")
    except (OSError, ValueError) as error:
        print(f"release report error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
