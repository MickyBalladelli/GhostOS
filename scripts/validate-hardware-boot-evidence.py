#!/usr/bin/env python3
"""Validate a captured SynOS bare-metal boot evidence bundle."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import sys
from datetime import datetime
from typing import Any


SCHEMA = 1
KIND = "synos-hardware-boot-evidence"
REVISION = re.compile(r"[0-9a-f]{7,64}")
REQUIRED_INVENTORY = (
    "environment",
    "hypervisor",
    "architecture",
    "boot_mode",
    "console",
    "machine_id",
    "firmware_vendor",
    "firmware_version",
    "motherboard",
    "cpu",
    "memory_mib",
    "storage",
    "nic",
)
REQUIRED_MARKERS = {
    "kernel_entry": "SynOS kernel bootstrap",
    "boot_info": "boot method=",
    "architecture": "architecture=x86_64",
    "cpu": "cpu topology online=",
    "pci": "PCI discovery complete",
    "user_handoff": "starting synos-init in Ring 3",
    "shell": "synos-shell ready in Ring 3",
}
PANIC_MARKERS = ("KERNEL PANIC", "guest panic", "SynOS boot failure")


def fail(errors: list[str], message: str) -> None:
    errors.append(message)


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def safe_relative_path(value: Any, name: str, errors: list[str]) -> pathlib.Path | None:
    if not isinstance(value, str) or not value.strip():
        fail(errors, f"{name} must be a non-empty relative path")
        return None
    path = pathlib.PurePosixPath(value)
    if path.is_absolute() or ".." in path.parts:
        fail(errors, f"{name} must stay inside the evidence directory")
        return None
    return pathlib.Path(*path.parts)


def parse_inventory(path: pathlib.Path, errors: list[str]) -> dict[str, str]:
    values: dict[str, str] = {}
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        fail(errors, f"cannot read inventory: {error}")
        return values

    for line_number, line in enumerate(lines, start=1):
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        key, separator, value = stripped.partition("=")
        if not separator or not key.strip() or not value.strip():
            fail(errors, f"inventory line {line_number} must use key=value")
            continue
        key = key.strip()
        if key in values:
            fail(errors, f"inventory contains duplicate key: {key}")
            continue
        values[key] = value.strip()

    for key in REQUIRED_INVENTORY:
        if not values.get(key):
            fail(errors, f"inventory is missing {key}")
    if values.get("environment") != "bare-metal":
        fail(errors, "inventory environment must be bare-metal")
    if values.get("hypervisor") not in {"none", "not-present"}:
        fail(errors, "inventory hypervisor must be none or not-present")
    if values.get("architecture") != "x86_64":
        fail(errors, "inventory architecture must be x86_64")
    if values.get("console") != "com1":
        fail(errors, "inventory console must be com1")
    memory = values.get("memory_mib", "")
    if not memory.isdigit() or int(memory) < 1:
        fail(errors, "inventory memory_mib must be a positive integer")
    return values


def parse_timestamp(value: Any, name: str, errors: list[str]) -> datetime | None:
    if not isinstance(value, str) or not value.strip():
        fail(errors, f"{name} must be a non-empty ISO-8601 timestamp")
        return None
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        fail(errors, f"{name} must be an ISO-8601 timestamp")
        return None
    if parsed.tzinfo is None:
        fail(errors, f"{name} must include a timezone")
    return parsed


def validate_file_record(
    evidence_dir: pathlib.Path,
    record: Any,
    name: str,
    errors: list[str],
) -> pathlib.Path | None:
    if not isinstance(record, dict):
        fail(errors, f"{name} must be an object")
        return None
    relative = safe_relative_path(record.get("path"), f"{name}.path", errors)
    digest = record.get("sha256")
    if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
        fail(errors, f"{name}.sha256 must be a SHA-256 digest")
    if relative is None:
        return None
    path = evidence_dir / relative
    if path.is_symlink():
        fail(errors, f"evidence files cannot be symlinks: {relative}")
        return None
    if not path.is_file():
        fail(errors, f"missing evidence file: {relative}")
        return None
    if isinstance(digest, str) and re.fullmatch(r"[0-9a-f]{64}", digest):
        if sha256(path) != digest:
            fail(errors, f"evidence digest does not match: {relative}")
    if isinstance(record.get("size"), int) and record["size"] != path.stat().st_size:
        fail(errors, f"evidence size does not match: {relative}")
    return path


def validate(evidence_dir: pathlib.Path, require_passed: bool = True) -> list[str]:
    errors: list[str] = []
    evidence_path = evidence_dir / "evidence.json"
    result_path = evidence_dir / "result.json"
    try:
        evidence = json.loads(evidence_path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        return [f"invalid evidence.json: {error}"]
    if not isinstance(evidence, dict):
        return ["evidence.json must contain an object"]

    if evidence.get("schema") != SCHEMA:
        fail(errors, f"evidence schema must be {SCHEMA}")
    if evidence.get("kind") != KIND:
        fail(errors, f"evidence kind must be {KIND}")
    if evidence.get("tier") != "hardware-boot":
        fail(errors, "evidence tier must be hardware-boot")
    state = evidence.get("result_state")
    if state not in {"passed", "failed", "skipped"}:
        fail(errors, "evidence result_state must be passed, failed, or skipped")
    if require_passed and state != "passed":
        fail(errors, "hardware boot evidence must be passed")
    if not isinstance(evidence.get("reason"), str) or not evidence["reason"].strip():
        fail(errors, "evidence reason is required")
    revision = evidence.get("revision")
    if not isinstance(revision, str) or not REVISION.fullmatch(revision):
        fail(errors, "evidence revision must be a Git revision")
    if evidence.get("platform") != "x86_64":
        fail(errors, "evidence platform must be x86_64")
    boot_mode = evidence.get("boot_mode")
    if boot_mode not in {"bios", "uefi"}:
        fail(errors, "evidence boot_mode must be bios or uefi")
    if evidence.get("environment") != "bare-metal":
        fail(errors, "evidence environment must be bare-metal")
    if evidence.get("hypervisor") != "none":
        fail(errors, "evidence hypervisor must be none")
    if not isinstance(evidence.get("operator"), str) or not evidence["operator"].strip():
        fail(errors, "evidence operator is required")
    capture_host = evidence.get("capture_host")
    if not isinstance(capture_host, dict) or any(
        not isinstance(capture_host.get(field), str) or not capture_host[field].strip()
        for field in ("system", "release", "architecture")
    ):
        fail(errors, "capture_host must name system, release, and architecture")
    if not isinstance(evidence.get("command"), str) or not evidence["command"].strip():
        fail(errors, "evidence command is required")

    started = parse_timestamp(evidence.get("started_at"), "started_at", errors)
    ended = parse_timestamp(evidence.get("ended_at"), "ended_at", errors)
    if started and ended and ended < started:
        fail(errors, "ended_at cannot be earlier than started_at")

    serial_record = evidence.get("serial_log")
    inventory_record = evidence.get("inventory")
    if isinstance(serial_record, dict) and serial_record.get("path") != "serial.log":
        fail(errors, "serial_log.path must be serial.log")
    if isinstance(inventory_record, dict) and inventory_record.get("path") != "inventory.txt":
        fail(errors, "inventory.path must be inventory.txt")
    serial_path = validate_file_record(evidence_dir, serial_record, "serial_log", errors)
    inventory_path = validate_file_record(evidence_dir, inventory_record, "inventory", errors)
    if serial_path:
        serial = serial_path.read_text(encoding="utf-8", errors="replace")
        markers = evidence.get("markers")
        if not isinstance(markers, dict):
            fail(errors, "evidence markers must be an object")
        for marker_name, marker_text in REQUIRED_MARKERS.items():
            if marker_text not in serial:
                fail(errors, f"serial log is missing marker {marker_name}: {marker_text}")
            elif isinstance(markers, dict) and markers.get(marker_name) != marker_text:
                fail(errors, f"evidence marker record is missing {marker_name}")
        for marker in PANIC_MARKERS:
            if marker.lower() in serial.lower():
                fail(errors, f"serial log contains failure marker: {marker}")
    if inventory_path:
        inventory = parse_inventory(inventory_path, errors)
        if boot_mode in {"bios", "uefi"} and inventory.get("boot_mode") != boot_mode:
            fail(errors, "inventory boot_mode does not match evidence boot_mode")

    artifacts = evidence.get("artifacts")
    if not isinstance(artifacts, list) or not artifacts:
        fail(errors, "evidence must contain at least one boot artifact")
    else:
        for index, artifact in enumerate(artifacts):
            artifact_path = validate_file_record(evidence_dir, artifact, f"artifacts[{index}]", errors)
            if isinstance(artifact, dict) and isinstance(artifact_path, pathlib.Path):
                if pathlib.PurePosixPath(artifact["path"]).parts[:1] != ("artifacts",):
                    fail(errors, f"artifacts[{index}].path must be under artifacts/")

    try:
        result = json.loads(result_path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        fail(errors, f"invalid result.json: {error}")
    else:
        if not isinstance(result, dict):
            fail(errors, "result.json must contain an object")
        else:
            if result.get("schema") != SCHEMA or result.get("kind") != f"{KIND}-result":
                fail(errors, "result.json has the wrong schema or kind")
            if result.get("state") != state:
                fail(errors, "result.json state does not match evidence result_state")
            if result.get("revision") != revision:
                fail(errors, "result.json revision does not match evidence")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence_dir", type=pathlib.Path)
    parser.add_argument("--allow-failed", action="store_true", help="validate structure without requiring passed")
    args = parser.parse_args()
    errors = validate(args.evidence_dir.expanduser().resolve(), require_passed=not args.allow_failed)
    if errors:
        print("hardware boot evidence validation failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print(f"hardware boot evidence valid: {args.evidence_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
