#!/usr/bin/env python3
"""Record host capability checks and run QEMU only when its prerequisites exist."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import platform
import shutil
import subprocess
from datetime import datetime, timezone


ROOT = pathlib.Path(__file__).resolve().parent.parent


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
    args = parser.parse_args()

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
