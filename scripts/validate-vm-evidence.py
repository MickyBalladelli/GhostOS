#!/usr/bin/env python3
"""Validate VM executed-evidence records against the inventory."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import sys
import tomllib


ROOT = pathlib.Path(__file__).resolve().parent.parent
INVENTORY_PATH = ROOT / "virtual_machine/tests/inventory.toml"
RESULT_STATES = {"passed", "failed", "skipped"}
SHA256 = re.compile(r"[0-9a-f]{64}")


def validate_tier_result(path: pathlib.Path, expected_tier: str, errors: list[str]) -> None:
    try:
        result = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        errors.append(f"{path}: invalid tier result JSON ({error})")
        return
    if not isinstance(result, dict):
        errors.append(f"{path}: tier result must be an object")
        return
    if result.get("schema") != 1:
        errors.append(f"{path}: schema must be 1")
    if result.get("tier") != expected_tier:
        errors.append(f"{path}: tier is {result.get('tier')!r}, expected {expected_tier!r}")
    state = result.get("state")
    if state not in RESULT_STATES:
        errors.append(f"{path}: invalid state {state!r}")
    reason = result.get("reason")
    if not isinstance(reason, str) or not reason.strip():
        errors.append(f"{path}: every result must contain a reason")
    if state == "failed" and (
        not isinstance(result.get("exit_code"), int) or result["exit_code"] == 0
    ):
        errors.append(f"{path}: failed result must contain a non-zero exit_code")


def validate_record(path: pathlib.Path, expected_id: str, expected_tier: str, errors: list[str]) -> None:
    try:
        record = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        errors.append(f"{path}: invalid evidence JSON ({error})")
        return

    expected = {
        "schema": 1,
        "test_id": expected_id,
        "tier": expected_tier,
    }
    for field, value in expected.items():
        if record.get(field) != value:
            errors.append(f"{path}: {field} is {record.get(field)!r}, expected {value!r}")
    for field in ("command", "revision", "firmware", "started_at", "ended_at", "reason"):
        if not isinstance(record.get(field), str) or not record[field].strip():
            errors.append(f"{path}: missing {field}")
    host = record.get("host")
    if not isinstance(host, dict) or any(not host.get(field) for field in ("system", "release", "architecture")):
        errors.append(f"{path}: host must name system, release, and architecture")
    if not isinstance(record.get("cpu_count"), int) or record["cpu_count"] < 1:
        errors.append(f"{path}: cpu_count must be positive")
    digest = record.get("image_digest")
    if digest != "not-applicable" and (not isinstance(digest, str) or not SHA256.fullmatch(digest)):
        errors.append(f"{path}: image_digest must be SHA-256 or 'not-applicable'")
    images = record.get("images")
    if not isinstance(images, list):
        errors.append(f"{path}: images must be a list")
    elif images:
        if any(
            not isinstance(image, dict)
            or not image.get("path")
            or not isinstance(image.get("sha256"), str)
            or not SHA256.fullmatch(image["sha256"])
            for image in images
        ):
            errors.append(f"{path}: every image must contain a path and SHA-256")
        elif len(images) == 1 and digest != images[0]["sha256"]:
            errors.append(f"{path}: image_digest does not match the recorded image")
        elif len(images) > 1:
            aggregate = hashlib.sha256()
            for image in images:
                aggregate.update(image["path"].encode())
                aggregate.update(b"\0")
                aggregate.update(image["sha256"].encode())
                aggregate.update(b"\0")
            if digest != aggregate.hexdigest():
                errors.append(f"{path}: image_digest does not match the recorded image set")
    elif digest != "not-applicable":
        errors.append(f"{path}: image_digest exists without image records")
    state = record.get("result_state")
    if state not in RESULT_STATES:
        errors.append(f"{path}: invalid result_state {state!r}")
    if state == "failed" and (
        not isinstance(record.get("exit_code"), int) or record["exit_code"] == 0
    ):
        errors.append(f"{path}: failed evidence must contain a non-zero exit_code")
    if state == "skipped" and not record.get("reason"):
        errors.append(f"{path}: skipped evidence must contain a reason")
    if expected_tier == "qemu" and state == "passed" and digest == "not-applicable":
        errors.append(f"{path}: passed QEMU evidence must contain an image digest")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence_dir", nargs="?", type=pathlib.Path)
    parser.add_argument("--schema-only", action="store_true")
    parser.add_argument("--require-tier", action="append", default=[])
    args = parser.parse_args()
    errors: list[str] = []

    try:
        inventory = tomllib.loads(INVENTORY_PATH.read_text())
    except (OSError, tomllib.TOMLDecodeError) as error:
        print(f"cannot load VM inventory: {error}", file=sys.stderr)
        return 1

    tier_names = {entry.get("name") for entry in inventory.get("tier", [])}
    tests = inventory.get("test", [])
    evidence = inventory.get("evidence", {})
    required_fields = {"command", "revision", "host", "firmware", "cpu_count", "image_digest", "result_state", "reason"}
    if set(evidence.get("required_fields", [])) != required_fields:
        errors.append("inventory evidence contract does not name every required execution field")
    if set(evidence.get("result_states", [])) != RESULT_STATES:
        errors.append("inventory evidence contract must use passed, failed, and skipped states")
    if evidence.get("record_schema") != 1:
        errors.append("inventory evidence contract must use record schema 1")
    if evidence.get("path") != "build/test-evidence/{run-id}/{tier}/{test-id}/evidence.json":
        errors.append("inventory evidence path does not match the executed record layout")
    for test in tests:
        if not test.get("id") or not test.get("tiers"):
            errors.append(f"inventory test lacks ID or tiers: {test!r}")
        for tier in test.get("tiers", []):
            if tier not in tier_names:
                errors.append(f"test {test.get('id')} references unknown tier {tier}")

    if not args.schema_only:
        if args.evidence_dir is None:
            parser.error("evidence_dir is required unless --schema-only is used")
        for tier in args.require_tier:
            if tier not in tier_names:
                errors.append(f"required unknown tier: {tier}")
                continue
            tier_result = args.evidence_dir / tier / "result.json"
            if not tier_result.is_file():
                errors.append(f"missing tier result: {tier_result}")
            else:
                validate_tier_result(tier_result, tier, errors)
            for test in tests:
                if tier not in test.get("tiers", []):
                    continue
                path = args.evidence_dir / tier / test["id"] / "evidence.json"
                if not path.is_file():
                    errors.append(f"missing executed evidence: {path}")
                    continue
                validate_record(path, test["id"], tier, errors)

    if errors:
        print("VM evidence validation failed:", file=sys.stderr)
        for item in errors:
            print(f"- {item}", file=sys.stderr)
        return 1
    print(f"VM evidence contract passed: {len(tests)} named behaviors")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
