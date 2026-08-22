#!/usr/bin/env python3
"""Reject release claims without fresh, complete SLO evidence."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import pathlib
import subprocess
import sys


ROOT = pathlib.Path(__file__).resolve().parent.parent
SLO_OBJECTIVE = 999_000
SLO_NAMES = (
    "boot",
    "interactive-shell",
    "ipc",
    "storage-commit",
    "dhcp",
    "rpc",
    "package-activation",
    "snapshot-restore",
    "cluster-convergence",
)


def git_revision() -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0 or not result.stdout.strip():
        raise ValueError("cannot determine current Git revision")
    return result.stdout.strip()


def parse_time(value: object, field: str) -> dt.datetime:
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{field} must be an ISO-8601 timestamp")
    try:
        parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise ValueError(f"{field} is not a valid ISO-8601 timestamp") from error
    if parsed.tzinfo is None:
        raise ValueError(f"{field} must include a timezone")
    return parsed.astimezone(dt.timezone.utc)


def allowed_bad_events(total: int, objective: int) -> int:
    return (total * (1_000_000 - objective) + 999_999) // 1_000_000


def evidence_path(evidence_dir: pathlib.Path, raw_path: object) -> pathlib.Path:
    if not isinstance(raw_path, str) or not raw_path.strip():
        raise ValueError("SLO evidence paths must be non-empty strings")
    relative = pathlib.PurePosixPath(raw_path)
    if relative.is_absolute() or ".." in relative.parts:
        raise ValueError(f"SLO evidence path escapes evidence directory: {raw_path!r}")
    candidate = evidence_dir / pathlib.Path(*relative.parts)
    if candidate.is_symlink():
        raise ValueError(f"SLO evidence file is missing or is a symlink: {raw_path}")
    path = candidate.resolve()
    try:
        path.relative_to(evidence_dir.resolve())
    except ValueError as error:
        raise ValueError(f"SLO evidence path escapes evidence directory: {raw_path!r}") from error
    if not path.is_file():
        raise ValueError(f"SLO evidence file is missing or is a symlink: {raw_path}")
    return path


def validate_evidence(
    evidence_dir: pathlib.Path,
    raw_paths: object,
    revision: str,
    now: dt.datetime,
    max_age: dt.timedelta,
) -> int:
    if not isinstance(raw_paths, list) or not raw_paths:
        raise ValueError("each SLO must cite at least one evidence file")
    newest_age = max_age
    for raw_path in raw_paths:
        path = evidence_path(evidence_dir, raw_path)
        try:
            record = json.loads(path.read_text())
        except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
            raise ValueError(f"SLO evidence is not valid JSON: {path}") from error
        if not isinstance(record, dict):
            raise ValueError(f"SLO evidence must be a JSON object: {path}")
        if record.get("revision") != revision:
            raise ValueError(f"SLO evidence revision does not match HEAD: {path}")
        state = record.get("result_state", record.get("state"))
        if state != "passed":
            raise ValueError(f"SLO evidence is not passed: {path}")
        ended_at = parse_time(
            record.get("ended_at", record.get("generated_at")),
            f"{path}: ended_at",
        )
        age = now - ended_at
        if age.total_seconds() < 0:
            raise ValueError(f"SLO evidence is dated in the future: {path}")
        if age > max_age:
            raise ValueError(f"SLO evidence is stale: {path}")
        newest_age = min(newest_age, age)
    return int(newest_age.total_seconds())


def validate_report(
    report_path: pathlib.Path,
    evidence_dir: pathlib.Path,
    now: dt.datetime,
    max_age: dt.timedelta,
) -> list[tuple[str, int, int, int]]:
    try:
        report = json.loads(report_path.read_text())
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read SLO report: {error}") from error
    if not isinstance(report, dict):
        raise ValueError("SLO report must be a JSON object")
    if report.get("schema") != 1 or report.get("kind") != "ghostos-slo-report":
        raise ValueError("SLO report has an unsupported schema")
    revision = git_revision()
    if report.get("revision") != revision:
        raise ValueError("SLO report revision does not match HEAD")
    generated_at = parse_time(report.get("generated_at"), "generated_at")
    age = now - generated_at
    if age.total_seconds() < 0:
        raise ValueError("SLO report is dated in the future")
    if age > max_age:
        raise ValueError("SLO report is stale")
    slos = report.get("slos")
    if not isinstance(slos, list) or len(slos) != len(SLO_NAMES):
        raise ValueError(f"SLO report must contain exactly {len(SLO_NAMES)} SLOs")

    by_name = {}
    for slo in slos:
        if not isinstance(slo, dict) or not isinstance(slo.get("name"), str):
            raise ValueError("every SLO report entry needs a name")
        name = slo["name"]
        if name in by_name or name not in SLO_NAMES:
            raise ValueError(f"unexpected or duplicate SLO: {name!r}")
        by_name[name] = slo

    results = []
    for name in SLO_NAMES:
        slo = by_name.get(name)
        if slo is None:
            raise ValueError(f"missing SLO: {name}")
        observed_at = parse_time(slo.get("observed_at"), f"{name}: observed_at")
        observed_age = now - observed_at
        if observed_age.total_seconds() < 0:
            raise ValueError(f"{name}: observation is dated in the future")
        if observed_age > max_age:
            raise ValueError(f"{name}: observation is stale")
        if slo.get("target_per_million") != SLO_OBJECTIVE:
            raise ValueError(f"{name}: unsupported objective")
        total = slo.get("total_events")
        bad = slo.get("bad_events")
        if not isinstance(total, int) or total <= 0 or not isinstance(bad, int) or bad < 0 or bad > total:
            raise ValueError(f"{name}: invalid event counts")
        allowed = allowed_bad_events(total, SLO_OBJECTIVE)
        remaining = max(0, allowed - bad)
        consumed = 0 if allowed == 0 and bad == 0 else (
            1_000_000 if allowed == 0 else min(1_000_000, bad * 1_000_000 // allowed)
        )
        if slo.get("allowed_bad_events") != allowed:
            raise ValueError(f"{name}: error budget allowance is wrong")
        if slo.get("remaining_bad_events") != remaining:
            raise ValueError(f"{name}: remaining error budget is wrong")
        if slo.get("consumed_per_million") != consumed:
            raise ValueError(f"{name}: error budget consumption is wrong")
        if slo.get("state") != "passed" or bad > allowed:
            raise ValueError(f"{name}: SLO is outside its error budget")
        evidence_age = validate_evidence(
            evidence_dir,
            slo.get("evidence"),
            revision,
            now,
            max_age,
        )
        results.append((name, consumed, bad, allowed))
        print(
            f"SLO {name}: budget {consumed}/1000000 consumed; "
            f"bad {bad}/{allowed}; evidence age {evidence_age}s"
        )
    return results


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", required=True, type=pathlib.Path)
    parser.add_argument("--evidence-dir", required=True, type=pathlib.Path)
    parser.add_argument("--max-age-seconds", type=int, default=86_400)
    parser.add_argument("--now", help="UTC ISO-8601 time for deterministic validation")
    args = parser.parse_args()
    if args.max_age_seconds <= 0:
        parser.error("--max-age-seconds must be positive")
    try:
        now = parse_time(args.now, "--now") if args.now else dt.datetime.now(dt.timezone.utc)
        results = validate_report(
            args.report.expanduser().resolve(),
            args.evidence_dir.expanduser().resolve(),
            now,
            dt.timedelta(seconds=args.max_age_seconds),
        )
    except (OSError, ValueError) as error:
        print(f"SLO release gate failed: {error}", file=sys.stderr)
        return 1
    print(f"SLO release gate passed: {len(results)} fresh budgets are within limits")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
