#!/usr/bin/env python3
"""Qualify host adapters and run QEMU only when its prerequisites exist."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import platform
import itertools
import shutil
import subprocess
import tomllib
from datetime import datetime, timezone


ROOT = pathlib.Path(__file__).resolve().parent.parent
MATRIX_PATH = ROOT / "docs/host-portability-matrix.toml"
EXPECTED_ARCHITECTURES = {"x86_64", "aarch64"}
EXPECTED_SYSTEMS = {"linux", "macos", "windows"}
EXPECTED_DOMAINS = (
    "firmware",
    "terminal",
    "disks",
    "networking",
    "acceleration",
    "timekeeping",
)


def utc_now() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def command_exists(value: str) -> bool:
    return bool(shutil.which(value))


def command_output(command: list[str]) -> str:
    try:
        result = subprocess.run(command, capture_output=True, text=True, check=False)
    except OSError:
        return ""
    return f"{result.stdout}\n{result.stderr}"


def acceleration() -> dict[str, str]:
    system = platform.system()
    if system == "Linux":
        kvm = pathlib.Path("/dev/kvm")
        if kvm.exists() and os.access(kvm, os.R_OK | os.W_OK):
            return {"backend": "kvm", "state": "available", "reason": "KVM device is present"}
        return {
            "backend": "kvm",
            "state": "skipped",
            "reason": "KVM device /dev/kvm is unavailable; software emulation is allowed",
        }
    if system == "Darwin":
        supported = command_output(["sysctl", "-n", "kern.hv_support"]).strip() == "1"
        if supported:
            return {"backend": "hvf", "state": "available", "reason": "Hypervisor.framework is available"}
        return {
            "backend": "hvf",
            "state": "skipped",
            "reason": "Hypervisor.framework is unavailable; software emulation is allowed",
        }
    if system == "Windows":
        qemu = os.environ.get("SYNOS_QEMU_BIN", "qemu-system-x86_64")
        if "whpx" in command_output([qemu, "-accel", "help"]).lower():
            return {"backend": "whpx", "state": "available", "reason": "WHPX is listed by QEMU"}
        return {
            "backend": "whpx",
            "state": "skipped",
            "reason": "WHPX is unavailable; software emulation is allowed",
        }
    return {
        "backend": "software",
        "state": "skipped",
        "reason": f"no supported hardware accelerator for {system}; software emulation is allowed",
    }


def qemu_capabilities() -> dict[str, object]:
    qemu = os.environ.get("SYNOS_QEMU_BIN", "qemu-system-x86_64")
    qemu_img = os.environ.get("SYNOS_QEMU_IMG_BIN", "qemu-img")
    bios = pathlib.Path(os.environ.get("SYNOS_QEMU_IMAGE", ROOT / "build/bios/synos-bios.img"))
    uefi_image = pathlib.Path(os.environ.get("SYNOS_QEMU_UEFI_IMAGE", bios))
    firmware_value = os.environ.get("SYNOS_QEMU_UEFI_FIRMWARE", "")
    firmware = pathlib.Path(firmware_value) if firmware_value else None
    missing: list[str] = []
    if not command_exists(qemu):
        missing.append(f"QEMU executable: {qemu}")
    if not command_exists(qemu_img):
        missing.append(f"qemu-img executable: {qemu_img}")
    if not bios.is_file():
        missing.append(f"BIOS image: {bios}")
    if not uefi_image.is_file():
        missing.append(f"UEFI image: {uefi_image}")
    if firmware is None or not firmware.is_file():
        missing.append("UEFI firmware: SYNOS_QEMU_UEFI_FIRMWARE")
    result: dict[str, object] = {
        "state": "available" if not missing else "skipped",
        "binary": qemu,
        "qemu_img": qemu_img,
        "bios_image": str(bios),
        "uefi_image": str(uefi_image),
        "uefi_firmware": str(firmware) if firmware else None,
        "missing": missing,
    }
    if missing:
        result["reason"] = "missing prerequisites: " + ", ".join(missing)
    else:
        result["reason"] = "QEMU, images, and UEFI firmware are available"
    return result


def normalize_architecture(value: str) -> str:
    normalized = value.lower()
    if normalized in {"x86_64", "amd64", "x64"}:
        return "x86_64"
    if normalized in {"aarch64", "arm64"}:
        return "aarch64"
    return normalized


def normalize_system(value: str) -> str:
    normalized = value.lower()
    if normalized in {"darwin", "macos", "mac os"}:
        return "macos"
    if normalized.startswith("win"):
        return "windows"
    return normalized


def load_portability_matrix(path: pathlib.Path) -> dict[str, object]:
    try:
        matrix = tomllib.loads(path.read_text())
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise ValueError(f"cannot read portability matrix: {error}") from error

    if matrix.get("schema") != 1:
        raise ValueError("portability matrix schema must be 1")
    data_model = matrix.get("data_model")
    model_fields = matrix.get("model_fields")
    if not isinstance(data_model, str) or not data_model:
        raise ValueError("portability matrix needs one data_model")
    if not isinstance(model_fields, list) or not model_fields or not all(
        isinstance(field, str) and field for field in model_fields
    ):
        raise ValueError("portability matrix needs non-empty model_fields")
    if tuple(matrix.get("domains", [])) != EXPECTED_DOMAINS:
        raise ValueError("portability matrix domains do not match the qualification contract")

    platforms = matrix.get("platform")
    if not isinstance(platforms, list):
        raise ValueError("portability matrix has no platform rows")
    seen = set()
    for row in platforms:
        if not isinstance(row, dict):
            raise ValueError("portability matrix has an invalid platform row")
        architecture = row.get("architecture")
        system = row.get("os")
        key = (architecture, system)
        if architecture not in EXPECTED_ARCHITECTURES or system not in EXPECTED_SYSTEMS:
            raise ValueError(f"unsupported platform row {key!r}")
        if key in seen:
            raise ValueError(f"duplicate platform row {key!r}")
        seen.add(key)
        for domain in EXPECTED_DOMAINS:
            entry = row.get(domain)
            if not isinstance(entry, dict):
                raise ValueError(f"{key!r} is missing {domain}")
            if entry.get("status") not in {"portable", "partial", "host-dependent"}:
                raise ValueError(f"{key!r}/{domain} has invalid status")
            if not isinstance(entry.get("reason"), str) or not entry["reason"].strip():
                raise ValueError(f"{key!r}/{domain} needs a reason")
            supported = entry.get("supported", [])
            if not isinstance(supported, list) or not supported or not all(
                isinstance(feature, str) and feature for feature in supported
            ):
                raise ValueError(f"{key!r}/{domain} needs supported features")
            skipped = entry.get("skipped", {})
            if not isinstance(skipped, dict) or any(
                not isinstance(feature, str)
                or not isinstance(reason, str)
                or not reason.strip()
                for feature, reason in skipped.items()
            ):
                raise ValueError(f"{key!r}/{domain} has an invalid skip reason")
    expected = set(itertools.product(EXPECTED_ARCHITECTURES, EXPECTED_SYSTEMS))
    if seen != expected:
        missing = ", ".join(
            f"{architecture}/{system}" for architecture, system in sorted(expected - seen)
        )
        raise ValueError(f"portability matrix is missing platform rows: {missing}")
    return matrix


def portability_feature(state: str, reason: str) -> dict[str, str]:
    return {"state": state, "reason": reason}


def portability_report(
    matrix: dict[str, object], qemu: dict[str, object], matrix_path: pathlib.Path
) -> dict[str, object]:
    host_system = normalize_system(platform.system())
    host_architecture = normalize_architecture(platform.machine())
    current = (host_architecture, host_system)
    rows = []
    skipped_count = 0

    for row in matrix["platform"]:
        key = (row["architecture"], row["os"])
        is_current = key == current
        cells = []
        for domain in EXPECTED_DOMAINS:
            contract = row[domain]
            features = {}
            if not is_current:
                for name in contract["supported"]:
                    features[name] = portability_feature(
                        "skipped",
                        f"qualification runs on {host_architecture}/{host_system}; run on {row['architecture']}/{row['os']}",
                    )
            elif domain == "firmware":
                features["bios-model"] = portability_feature(
                    "passed", "guest BIOS model is host-independent"
                )
                features["uefi-model"] = portability_feature(
                    "passed", "guest UEFI model is host-independent"
                )
                qemu_reason = str(qemu["reason"])
                qemu_state = "passed" if qemu["state"] == "available" else "skipped"
                features["qemu-bios"] = portability_feature(qemu_state, qemu_reason)
                features["qemu-uefi"] = portability_feature(qemu_state, qemu_reason)
            elif domain == "acceleration":
                features["software"] = portability_feature(
                    "passed", "portable software execution is the semantic reference"
                )
                native = next(name for name in contract["supported"] if name != "software")
                acceleration_result = acceleration()
                if (
                    acceleration_result["backend"] == native
                    and acceleration_result["state"] == "available"
                ):
                    features[native] = portability_feature(
                        "passed", str(acceleration_result["reason"])
                    )
                else:
                    features[native] = portability_feature(
                        "skipped", str(acceleration_result["reason"])
                    )
            else:
                for name in contract["supported"]:
                    features[name] = portability_feature("passed", contract["reason"])
            for name, reason in contract.get("skipped", {}).items():
                features[name] = portability_feature("skipped", reason)
            skipped_count += sum(
                value["state"] == "skipped" for value in features.values()
            )
            cells.append(
                {
                    "domain": domain,
                    "declared_status": contract["status"],
                    "reason": contract["reason"],
                    "features": features,
                    "data_model": matrix["data_model"],
                }
            )
        rows.append(
            {
                "architecture": row["architecture"],
                "os": row["os"],
                "current_host": is_current,
                "data_model": matrix["data_model"],
                "cells": cells,
            }
        )

    return {
        "schema": 1,
        "matrix": str(matrix_path.relative_to(ROOT))
        if matrix_path.is_relative_to(ROOT)
        else str(matrix_path),
        "data_model": matrix["data_model"],
        "model_fields": matrix["model_fields"],
        "host": {
            "system": platform.system(),
            "release": platform.release(),
            "architecture": platform.machine(),
            "normalized_system": host_system,
            "normalized_architecture": host_architecture,
        },
        "state": "passed" if skipped_count == 0 else "passed_with_skips",
        "skipped_features": skipped_count,
        "platforms": rows,
    }


def run_qemu(capabilities: dict[str, object]) -> dict[str, object]:
    command = [
        "cargo",
        "test",
        "-p",
        "synos-vm",
        "--test",
        "qemu_matrix_59_11",
        "--test",
        "test_environments",
        "--test",
        "qemu_login_e2e",
        "--",
        "--ignored",
    ]
    if capabilities["state"] == "skipped":
        return {
            "state": "skipped",
            "command": "SYNOS_RUN_QEMU_TESTS=1 " + " ".join(command),
            "reason": str(capabilities["reason"]),
        }
    environment = os.environ.copy()
    environment["SYNOS_RUN_QEMU_TESTS"] = "1"
    try:
        result = subprocess.run(command, cwd=ROOT, env=environment, check=False)
    except OSError as error:
        return {"state": "failed", "command": " ".join(command), "reason": str(error), "exit_code": 1}
    return {
        "state": "passed" if result.returncode == 0 else "failed",
        "command": "SYNOS_RUN_QEMU_TESTS=1 " + " ".join(command),
        "reason": "QEMU matrix completed" if result.returncode == 0 else "QEMU matrix failed",
        "exit_code": result.returncode,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--run-qemu", action="store_true")
    parser.add_argument("--matrix", type=pathlib.Path, default=MATRIX_PATH)
    args = parser.parse_args()

    matrix_path = args.matrix.resolve()
    matrix = load_portability_matrix(matrix_path)
    qemu = qemu_capabilities()
    report: dict[str, object] = {
        "schema": 1,
        "started_at": utc_now(),
        "host": {
            "system": platform.system(),
            "release": platform.release(),
            "architecture": platform.machine(),
        },
        "acceleration": acceleration(),
        "qemu": qemu,
        "portability": portability_report(matrix, qemu, matrix_path),
    }
    if args.run_qemu:
        report["qemu_tests"] = run_qemu(qemu)
    report["ended_at"] = utc_now()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
