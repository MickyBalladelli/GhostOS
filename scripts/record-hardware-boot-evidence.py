#!/usr/bin/env python3
"""Package a successful GhostOS boot captured from physical x86_64 hardware."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import platform
import shlex
import shutil
import subprocess
import sys
from datetime import datetime, timezone


ROOT = pathlib.Path(__file__).resolve().parent.parent
VALIDATOR = ROOT / "scripts/validate-hardware-boot-evidence.py"
MARKERS = {
    "kernel_entry": "GhostOS kernel bootstrap",
    "boot_info": "boot method=",
    "architecture": "architecture=x86_64",
    "cpu": "cpu topology online=",
    "pci": "PCI discovery complete",
    "user_handoff": "starting ghostos-init in Ring 3",
    "shell": "ghostos-shell ready in Ring 3",
}


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def timestamp(value: str | None) -> str:
    if value:
        return value
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def git_revision() -> str | None:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True, check=False
    )
    revision = result.stdout.strip()
    return revision or None


def inventory_values(path: pathlib.Path) -> dict[str, str]:
    values = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        key, separator, value = stripped.partition("=")
        if separator:
            values[key.strip()] = value.strip()
    return values


def file_record(path: pathlib.Path, relative: str) -> dict[str, object]:
    return {
        "path": relative,
        "size": path.stat().st_size,
        "sha256": sha256(path),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence-dir", required=True, type=pathlib.Path)
    parser.add_argument("--serial-log", required=True, type=pathlib.Path)
    parser.add_argument("--inventory", required=True, type=pathlib.Path)
    parser.add_argument("--artifact", required=True, action="append", type=pathlib.Path)
    parser.add_argument("--boot-mode", required=True, choices=("bios", "uefi"))
    parser.add_argument("--operator", required=True)
    parser.add_argument("--revision", help="Git revision used to build the boot artifact")
    parser.add_argument("--started-at")
    parser.add_argument("--ended-at")
    args = parser.parse_args()

    serial_log = args.serial_log.expanduser().resolve()
    inventory = args.inventory.expanduser().resolve()
    artifacts = [path.expanduser().resolve() for path in args.artifact]
    evidence_dir = args.evidence_dir.expanduser().resolve()
    inputs = [serial_log, inventory, *artifacts]
    if any(not path.is_file() for path in inputs):
        missing = [str(path) for path in inputs if not path.is_file()]
        print(f"missing evidence input: {', '.join(missing)}", file=sys.stderr)
        return 1
    if not args.operator.strip():
        parser.error("--operator must not be empty")
    if len({path.name for path in artifacts}) != len(artifacts):
        parser.error("--artifact names must be unique")
    if evidence_dir.exists() and (not evidence_dir.is_dir() or any(evidence_dir.iterdir())):
        print(f"evidence directory is not empty: {evidence_dir}", file=sys.stderr)
        return 1

    values = inventory_values(inventory)
    serial = serial_log.read_text(encoding="utf-8", errors="replace")
    missing_markers = [name for name, marker in MARKERS.items() if marker not in serial]
    if missing_markers:
        print(f"serial log is missing markers: {', '.join(missing_markers)}", file=sys.stderr)
        return 1
    if any(marker.lower() in serial.lower() for marker in ("KERNEL PANIC", "guest panic", "GhostOS boot failure")):
        print("serial log contains a boot failure marker", file=sys.stderr)
        return 1
    if values.get("boot_mode") != args.boot_mode:
        print("inventory boot_mode does not match --boot-mode", file=sys.stderr)
        return 1
    if values.get("environment") != "bare-metal" or values.get("hypervisor") not in {"none", "not-present"}:
        print("inventory must declare environment=bare-metal and no hypervisor", file=sys.stderr)
        return 1

    revision = args.revision or git_revision()
    if not revision:
        print("a Git revision is required; pass --revision", file=sys.stderr)
        return 1

    evidence_dir.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(serial_log, evidence_dir / "serial.log")
    shutil.copyfile(inventory, evidence_dir / "inventory.txt")
    artifact_dir = evidence_dir / "artifacts"
    artifact_dir.mkdir()
    artifact_records = []
    for artifact in artifacts:
        destination = artifact_dir / artifact.name
        shutil.copyfile(artifact, destination)
        artifact_records.append(file_record(destination, f"artifacts/{artifact.name}"))

    serial_record = file_record(evidence_dir / "serial.log", "serial.log")
    inventory_record = file_record(evidence_dir / "inventory.txt", "inventory.txt")
    evidence = {
        "schema": 1,
        "kind": "ghostos-hardware-boot-evidence",
        "test_id": "hardware.boot.x86_64",
        "tier": "hardware-boot",
        "result_state": "passed",
        "reason": f"bare-metal {args.boot_mode} boot reached the Ring 3 shell",
        "revision": revision,
        "platform": "x86_64",
        "boot_mode": args.boot_mode,
        "environment": "bare-metal",
        "hypervisor": "none",
        "operator": args.operator.strip(),
        "started_at": timestamp(args.started_at),
        "ended_at": timestamp(args.ended_at),
        "capture_host": {
            "system": platform.system(),
            "release": platform.release(),
            "architecture": platform.machine(),
        },
        "command": shlex.join(sys.argv),
        "serial_log": serial_record,
        "inventory": inventory_record,
        "artifacts": artifact_records,
        "markers": MARKERS,
    }
    (evidence_dir / "evidence.json").write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n")
    result = {
        "schema": 1,
        "kind": "ghostos-hardware-boot-evidence-result",
        "tier": "hardware-boot",
        "state": "passed",
        "revision": revision,
        "reason": f"bare-metal {args.boot_mode} boot reached the Ring 3 shell",
    }
    (evidence_dir / "result.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")

    validation = subprocess.run([sys.executable, str(VALIDATOR), str(evidence_dir)], check=False)
    if validation.returncode:
        return validation.returncode
    print(f"recorded hardware boot evidence: {evidence_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
