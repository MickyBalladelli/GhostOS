#!/usr/bin/env python3
"""Resolve roadmap test plans into honest execution statuses."""

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
STATES = ("planned", "running", "passed", "failed", "blocked")
EVIDENCE_STATES = {"passed", "failed", "blocked"}
STATE_ALIASES = {"pass": "passed", "fail": "failed", "skipped": "blocked"}


def load_inventory(errors: list[str]) -> dict:
    try:
        inventory = tomllib.loads(INVENTORY_PATH.read_text())
    except (OSError, tomllib.TOMLDecodeError) as exc:
        errors.append(f"cannot load {INVENTORY_PATH.relative_to(ROOT)}: {exc}")
        return {}

    status = inventory.get("status", {})
    if status.get("values") != list(STATES):
        errors.append("docs/test-inventory.toml: status.values must list the five roadmap states")
    if status.get("default") != "planned":
        errors.append("docs/test-inventory.toml: status.default must be 'planned'")
    if set(status.get("evidence_values", [])) != EVIDENCE_STATES:
        errors.append(
            "docs/test-inventory.toml: status.evidence_values must be passed, failed, and blocked"
        )
    if status.get("named_test_policy") != "plan-only":
        errors.append("docs/test-inventory.toml: named_test_policy must be 'plan-only'")
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


def read_status_record(path: Path, errors: list[str], evidence: bool) -> tuple[str, str] | None:
    value = read_json(path, errors)
    if value is None:
        return None
    raw_state = value.get("result_state", value.get("state"))
    state = STATE_ALIASES.get(raw_state, raw_state) if isinstance(raw_state, str) else None
    if state not in STATES:
        errors.append(f"{path}: invalid state {raw_state!r}")
        return None
    if evidence and state not in EVIDENCE_STATES:
        errors.append(f"{path}: evidence must use passed, failed, or blocked, not {state}")
        return None
    if not isinstance(value.get("reason"), str) or not value["reason"].strip():
        errors.append(f"{path}: status requires a non-empty reason")
    if evidence and state in {"passed", "failed"} and "revision" not in value:
        errors.append(f"{path}: {state} evidence requires a source revision")
    return state, path.as_posix()


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
            result["state"], result["evidence"] = record
        return result
    if status_path.is_file():
        record = read_status_record(status_path, errors, evidence=False)
        if record is not None:
            state, source = record
            if state in {"passed", "failed"}:
                errors.append(f"{status_path}: {state} requires evidence.json, not status.json")
            else:
                result["state"], result["evidence"] = state, source
    return result


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
    report = {
        "schema": 1,
        "source": "docs/test-inventory.toml",
        "named_test_policy": "plan-only",
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "counts": {state: counts.get(state, 0) for state in STATES},
        "tests": statuses,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(
        f"test status valid: {len(statuses)} named tests; "
        + ", ".join(f"{state}={counts.get(state, 0)}" for state in STATES)
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
