#!/usr/bin/env python3
"""Resolve roadmap test plans into honest test and feature statuses."""

from __future__ import annotations

import argparse
import json
import sys
import tomllib
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
INVENTORY_PATH = ROOT / "docs" / "test-inventory.toml"
TIERS = ("unit", "integration", "qemu", "fault", "fuzz", "performance")
STATES = (
    "planned",
    "running",
    "passed",
    "failed",
    "blocked",
    "skipped",
    "inconclusive",
)
EVIDENCE_STATES = {"passed", "failed", "blocked", "skipped", "inconclusive"}
STATE_ALIASES = {"pass": "passed", "fail": "failed"}
EVIDENCE_TEXT_FIELDS = ("test_id", "command", "revision", "started_at", "ended_at", "reason")


def load_inventory(errors: list[str]) -> dict:
    try:
        inventory = tomllib.loads(INVENTORY_PATH.read_text())
    except (OSError, tomllib.TOMLDecodeError) as exc:
        errors.append(f"cannot load {INVENTORY_PATH.relative_to(ROOT)}: {exc}")
        return {}

    status = inventory.get("status", {})
    if status.get("values") != list(STATES):
        errors.append("docs/test-inventory.toml: status.values must list the seven roadmap states")
    if status.get("default") != "planned":
        errors.append("docs/test-inventory.toml: status.default must be 'planned'")
    if set(status.get("evidence_values", [])) != EVIDENCE_STATES:
        errors.append(
            "docs/test-inventory.toml: status.evidence_values must be passed, failed, blocked, skipped, and inconclusive"
        )
    if status.get("named_test_policy") != "plan-only":
        errors.append("docs/test-inventory.toml: named_test_policy must be 'plan-only'")
    if not isinstance(status.get("default_owner"), str) or not status["default_owner"].strip():
        errors.append("docs/test-inventory.toml: status.default_owner must be non-empty")
    if not isinstance(status.get("stale_after_days"), int) or status["stale_after_days"] <= 0:
        errors.append("docs/test-inventory.toml: status.stale_after_days must be positive")
    return inventory


def inventory_tests(inventory: dict, errors: list[str]) -> list[dict[str, str]]:
    tests: dict[str, dict[str, str]] = {}
    for feature in inventory.get("feature", []):
        feature_id = str(feature.get("id", "")).zfill(2)
        for tier in TIERS:
            for test_id in feature.get(tier, []):
                if not isinstance(test_id, str):
                    errors.append(f"feature {feature_id} has a non-string {tier} test ID")
                    continue
                previous = tests.get(test_id)
                if previous is not None:
                    errors.append(
                        f"test ID {test_id} is mapped more than once "
                        f"({previous['feature_id']} and {feature_id})"
                    )
                    continue
                tests[test_id] = {
                    "test_id": test_id,
                    "feature_id": feature_id,
                    "tier": tier,
                }
    return [tests[test_id] for test_id in sorted(tests)]


def read_json(path: Path, errors: list[str]) -> dict | None:
    try:
        value = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        errors.append(f"{path}: invalid JSON ({exc})")
        return None
    if not isinstance(value, dict):
        errors.append(f"{path}: status record must be an object")
        return None
    return value


def read_status_record(path: Path, errors: list[str], evidence: bool) -> dict[str, str] | None:
    value = read_json(path, errors)
    if value is None:
        return None
    raw_state = value.get("result_state", value.get("state"))
    state = STATE_ALIASES.get(raw_state, raw_state) if isinstance(raw_state, str) else None
    if state not in STATES:
        errors.append(f"{path}: invalid state {raw_state!r}")
        return None
    if evidence and state not in EVIDENCE_STATES:
        errors.append(
            f"{path}: evidence must use passed, failed, blocked, skipped, or inconclusive, not {state}"
        )
        return None
    if not isinstance(value.get("reason"), str) or not value["reason"].strip():
        errors.append(f"{path}: status requires a non-empty reason")
    if evidence:
        for field in EVIDENCE_TEXT_FIELDS:
            field_value = value.get(field)
            if not isinstance(field_value, str) or not field_value.strip():
                errors.append(f"{path}: evidence requires a non-empty {field}")
        host = value.get("host")
        if not isinstance(host, (str, dict)) or not host:
            errors.append(f"{path}: evidence requires a non-empty host")
        if value.get("revision") in {None, "", "unknown"}:
            errors.append(f"{path}: evidence requires a real source revision")
        for field in ("started_at", "ended_at"):
            timestamp = value.get(field)
            if isinstance(timestamp, str):
                try:
                    datetime.fromisoformat(timestamp.replace("Z", "+00:00"))
                except ValueError:
                    errors.append(f"{path}: {field} is not an ISO-8601 timestamp")
    if evidence and state in {"passed", "failed"} and "revision" not in value:
        errors.append(f"{path}: {state} evidence requires a source revision")
    record = {
        "state": state,
        "evidence": path.as_posix(),
    }
    for field in ("ended_at", "updated_at", "started_at", "reason", "prerequisite"):
        field_value = value.get(field)
        if isinstance(field_value, str) and field_value.strip():
            record[field] = field_value.strip()
    return record


def parse_timestamp(value: str | None) -> datetime | None:
    if not value:
        return None
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
        return parsed if parsed.tzinfo is not None else parsed.replace(tzinfo=timezone.utc)
    except ValueError:
        return None


def resolve_status(test: dict[str, str], evidence_dir: Path | None, errors: list[str]) -> dict[str, str]:
    result = {**test, "state": "planned"}
    if evidence_dir is None:
        return result

    test_dir = evidence_dir / test["tier"] / test["test_id"]
    evidence_path = test_dir / "evidence.json"
    status_path = test_dir / "status.json"
    if evidence_path.is_file():
        record = read_status_record(evidence_path, errors, evidence=True)
        if record is not None:
            result.update(record)
        return result
    if status_path.is_file():
        record = read_status_record(status_path, errors, evidence=False)
        if record is not None:
            if record["state"] in {"passed", "failed", "skipped", "inconclusive"}:
                errors.append(
                    f"{status_path}: {record['state']} requires evidence.json, not status.json"
                )
            else:
                result.update(record)
    if evidence_dir is not None and "evidence" not in result:
        tier_result_path = evidence_dir / test["tier"] / "result.json"
        if tier_result_path.is_file():
            tier_result = read_json(tier_result_path, errors)
            if tier_result:
                tier_state = tier_result.get("result_state", tier_result.get("state"))
                if tier_state in {"skipped", "inconclusive"}:
                    result["state"] = tier_state
                    reason = tier_result.get("reason")
                    if isinstance(reason, str) and reason.strip():
                        result["reason"] = reason.strip()
                    prerequisite = tier_result.get("prerequisite")
                    if isinstance(prerequisite, str) and prerequisite.strip():
                        result["prerequisite"] = prerequisite.strip()
    return result


def aggregate_feature_status(statuses: list[dict[str, str]]) -> str:
    states = {status["state"] for status in statuses}
    if "failed" in states:
        return "failed"
    if "inconclusive" in states:
        return "inconclusive"
    if "blocked" in states:
        return "blocked"
    if "running" in states:
        return "running"
    if "planned" in states:
        return "planned"
    if "skipped" in states:
        return "skipped"
    if statuses and states == {"passed"}:
        return "passed"
    return "planned"


def feature_report(inventory: dict, statuses: list[dict[str, str]], now: datetime) -> list[dict[str, object]]:
    status_config = inventory.get("status", {})
    default_owner = status_config.get("default_owner", "unassigned")
    stale_after_days = status_config.get("stale_after_days", 30)
    by_feature: dict[str, list[dict[str, str]]] = {}
    for status in statuses:
        by_feature.setdefault(status["feature_id"], []).append(status)

    report = []
    for feature in inventory.get("feature", []):
        feature_id = str(feature.get("id", "")).zfill(2)
        feature_statuses = by_feature.get(feature_id, [])
        evidence_times = [
            parsed
            for status in feature_statuses
            if (parsed := parse_timestamp(status.get("ended_at"))) is not None
        ]
        last_evidence_time = max(evidence_times) if evidence_times else None
        evidence_age_days = (
            round(max(0.0, (now - last_evidence_time).total_seconds() / 86400), 2)
            if last_evidence_time
            else None
        )
        prerequisites = sorted(
            {
                status["prerequisite"]
                for status in feature_statuses
                if status.get("prerequisite")
            }
        )
        report.append(
            {
                "feature_id": feature_id,
                "feature": feature.get("todo_heading", ""),
                "code_owner": feature.get("owner", default_owner),
                "state": aggregate_feature_status(feature_statuses),
                "test_count": len(feature_statuses),
                "last_evidence": last_evidence_time.isoformat() if last_evidence_time else None,
                "evidence_age_days": evidence_age_days,
                "evidence_stale": evidence_age_days is not None and evidence_age_days > stale_after_days,
                "stale_after_days": stale_after_days,
                "skipped_prerequisites": prerequisites,
                "test_ids": [status["test_id"] for status in feature_statuses],
            }
        )
    return report


def write_markdown(path: Path, report: dict[str, object]) -> None:
    def cell(value: object) -> str:
        return str(value if value not in (None, "") else "—").replace("|", "\\|")

    lines = [
        "# GhostOS roadmap status",
        "",
        f"Generated: `{report['generated_at']}`",
        f"Evidence age threshold: `{report['stale_after_days']} days`",
        "",
        "| ID | Feature | Code owner | State | Last evidence | Age (days) | Skipped prerequisites |",
        "| --- | --- | --- | --- | --- | ---: | --- |",
    ]
    for feature in report["features"]:
        prerequisites = ", ".join(feature["skipped_prerequisites"]) or "—"
        age = feature["evidence_age_days"]
        if feature["evidence_stale"]:
            age = f"{age} (stale)"
        lines.append(
            "| "
            + " | ".join(
                [
                    cell(feature["feature_id"]),
                    cell(feature["feature"]),
                    cell(feature["code_owner"]),
                    cell(feature["state"]),
                    cell(feature["last_evidence"]),
                    cell(age),
                    cell(prerequisites),
                ]
            )
            + " |"
        )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--evidence-dir",
        type=Path,
        help="run evidence root; absent per-test evidence leaves tests planned",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "build/test-status.json",
        help="JSON status report path",
    )
    parser.add_argument(
        "--markdown-output",
        type=Path,
        help="Markdown feature status report path; defaults beside --output",
    )
    args = parser.parse_args()

    errors: list[str] = []
    inventory = load_inventory(errors)
    tests = inventory_tests(inventory, errors)
    statuses = [resolve_status(test, args.evidence_dir, errors) for test in tests]

    if errors:
        for error in errors:
            print(f"test status error: {error}", file=sys.stderr)
        return 1

    counts = Counter(status["state"] for status in statuses)
    generated_at = datetime.now(timezone.utc)
    features = feature_report(inventory, statuses, generated_at)
    feature_counts = Counter(feature["state"] for feature in features)
    status_config = inventory.get("status", {})
    report = {
        "schema": 1,
        "source": "docs/test-inventory.toml",
        "named_test_policy": "plan-only",
        "status_values": list(STATES),
        "evidence_values": [state for state in STATES if state in EVIDENCE_STATES],
        "generated_at": generated_at.isoformat(),
        "stale_after_days": status_config.get("stale_after_days", 30),
        "counts": {state: counts.get(state, 0) for state in STATES},
        "feature_counts": {state: feature_counts.get(state, 0) for state in STATES},
        "tests": statuses,
        "features": features,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    markdown_output = args.markdown_output or args.output.with_suffix(".md")
    write_markdown(markdown_output, report)
    print(
        f"test status valid: {len(statuses)} named tests; "
        + ", ".join(f"{state}={counts.get(state, 0)}" for state in STATES)
        + f"; {len(features)} feature reports written"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
