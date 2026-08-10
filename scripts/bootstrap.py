#!/usr/bin/env python3
"""Check the reproducible SynOS developer prerequisites."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import platform
import shutil
import subprocess
import sys
import tomllib


ROOT = pathlib.Path(__file__).resolve().parent.parent
TOOLCHAIN_FILE = ROOT / "rust-toolchain.toml"
IMAGE_ARTIFACTS = (
    ("BIOS image", ROOT / "build/bios/synos-bios.img"),
    ("portable UEFI image", ROOT / "build/portable/synos.img"),
)


def run(command: list[str]) -> tuple[int, str]:
    try:
        result = subprocess.run(
            command,
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError as error:
        return 127, str(error)
    output = (result.stdout + "\n" + result.stderr).strip()
    return result.returncode, output


def command_version(command: str, arguments: list[str] | None = None) -> str:
    code, output = run([command, *(arguments or ["--version"])])
    if code:
        return ""
    return output.splitlines()[0] if output else ""


def command_exists(command: str) -> bool:
    return shutil.which(command) is not None


def add_check(
    checks: list[dict[str, object]],
    name: str,
    state: str,
    detail: str,
    required: bool,
) -> None:
    checks.append(
        {
            "name": name,
            "state": state,
            "detail": detail,
            "required": required,
        }
    )


def installed_items(command: list[str]) -> set[str]:
    code, output = run(command)
    if code:
        return set()
    return {
        line.split()[0]
        for line in output.splitlines()
        if line.strip() and not line.startswith("info:")
    }


def check_toolchain(checks: list[dict[str, object]]) -> dict[str, object]:
    try:
        config = tomllib.loads(TOOLCHAIN_FILE.read_text())
        toolchain = config["toolchain"]
    except (OSError, tomllib.TOMLDecodeError, KeyError) as error:
        add_check(checks, "rust-toolchain.toml", "failed", str(error), True)
        return {}

    expected_channel = str(toolchain["channel"])
    add_check(
        checks,
        "rust-toolchain.toml",
        "passed",
        f"channel {expected_channel}, profile {toolchain.get('profile', 'default')}",
        True,
    )

    for command in ("rustup", "rustc", "cargo", "clang", "rg", "git"):
        version = command_version(command)
        add_check(
            checks,
            command,
            "passed" if version else "failed",
            version or "command is not available",
            True,
        )

    if command_exists("rustup"):
        code, active = run(["rustup", "show", "active-toolchain"])
        active_name = active.split()[0] if code == 0 and active else ""
        matches = active_name == expected_channel or active_name.startswith(
            expected_channel + "-"
        )
        add_check(
            checks,
            "active Rust toolchain",
            "passed" if matches else "failed",
            active_name or "rustup did not report an active toolchain",
            True,
        )

        required_components = set(toolchain.get("components", []))
        installed_components = installed_items(
            ["rustup", "component", "list", "--installed"]
        )
        missing_components = [
            component
            for component in sorted(required_components)
            if not any(
                installed == component or installed.startswith(component + "-")
                for installed in installed_components
            )
        ]
        add_check(
            checks,
            "Rust components",
            "passed" if not missing_components else "failed",
            ", ".join(sorted(required_components))
            if not missing_components
            else "missing " + ", ".join(missing_components),
            True,
        )

        required_targets = set(toolchain.get("targets", []))
        installed_targets = installed_items(["rustup", "target", "list", "--installed"])
        missing_targets = sorted(required_targets - installed_targets)
        add_check(
            checks,
            "Rust targets",
            "passed" if not missing_targets else "failed",
            ", ".join(sorted(required_targets))
            if not missing_targets
            else "missing " + ", ".join(missing_targets),
            True,
        )
    else:
        add_check(
            checks,
            "active Rust toolchain",
            "failed",
            "rustup is required to verify the pinned components and targets",
            True,
        )

    rustc = shutil.which("rustc")
    if rustc:
        code, target_libdir = run(["rustc", "--print", "target-libdir"])
        rust_tools_dir = pathlib.Path(target_libdir).parent / "bin" if code == 0 else None
        for tool in ("rust-lld", "llvm-objcopy"):
            path = rust_tools_dir / tool if rust_tools_dir else None
            add_check(
                checks,
                f"bundled {tool}",
                "passed" if path and path.is_file() else "failed",
                str(path) if path and path.is_file() else "bundled LLVM tool is missing",
                True,
            )

    return toolchain


def check_custom_targets(checks: list[dict[str, object]]) -> None:
    targets = sorted((ROOT / "targets").glob("*.json"))
    if not targets:
        add_check(checks, "custom target specifications", "failed", "no target JSON files found", True)
        return
    invalid = []
    for path in targets:
        try:
            target = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError) as error:
            invalid.append(f"{path.name}: {error}")
            continue
        if not target.get("llvm-target") or not target.get("arch"):
            invalid.append(f"{path.name}: missing llvm-target or arch")
    add_check(
        checks,
        "custom target specifications",
        "passed" if not invalid else "failed",
        ", ".join(path.name for path in targets)
        if not invalid
        else "; ".join(invalid),
        True,
    )


def check_qemu(checks: list[dict[str, object]], require_qemu: bool) -> None:
    qemu = os.environ.get("SYNOS_QEMU_BIN", "qemu-system-x86_64")
    qemu_img = os.environ.get("SYNOS_QEMU_IMG_BIN", "qemu-img")
    missing = [
        command
        for command in (qemu, qemu_img)
        if not command_exists(command)
    ]
    if missing:
        add_check(
            checks,
            "QEMU tools",
            "failed" if require_qemu else "skipped",
            "missing " + ", ".join(missing),
            require_qemu,
        )
        return
    versions = [command_version(qemu), command_version(qemu_img)]
    add_check(checks, "QEMU tools", "passed", "; ".join(versions), require_qemu)


def git_revision() -> str:
    code, output = run(["git", "rev-parse", "HEAD"])
    return output.splitlines()[0] if code == 0 and output else ""


def check_images(
    checks: list[dict[str, object]],
    require_images: bool,
    revision: str,
) -> None:
    for name, image in IMAGE_ARTIFACTS:
        if not image.exists():
            add_check(
                checks,
                name,
                "failed" if require_images else "skipped",
                f"not built: {image.relative_to(ROOT)}",
                require_images,
            )
            continue
        sidecar = pathlib.Path(str(image) + ".revision")
        recorded = sidecar.read_text().strip() if sidecar.is_file() else ""
        if not recorded:
            add_check(
                checks,
                name,
                "failed",
                f"missing revision sidecar: {sidecar.relative_to(ROOT)}",
                True,
            )
        elif not revision:
            add_check(checks, name, "failed", "cannot determine Git revision", True)
        elif recorded != revision:
            add_check(
                checks,
                name,
                "failed",
                f"image revision {recorded} does not match HEAD {revision}",
                True,
            )
        else:
            add_check(checks, name, "passed", f"revision {revision}", True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--require-qemu",
        action="store_true",
        help="turn optional QEMU prerequisites into required checks",
    )
    parser.add_argument(
        "--require-images",
        action="store_true",
        help="require both built images and matching revision sidecars",
    )
    parser.add_argument(
        "--json",
        dest="json_path",
        type=pathlib.Path,
        help="also write the deterministic check report to this path",
    )
    args = parser.parse_args()

    checks: list[dict[str, object]] = []
    check_toolchain(checks)
    check_custom_targets(checks)
    check_qemu(checks, args.require_qemu)
    revision = git_revision()
    add_check(
        checks,
        "Git revision",
        "passed" if revision else "failed",
        revision or "cannot determine HEAD",
        True,
    )
    check_images(checks, args.require_images, revision)

    report = {
        "schema": 1,
        "host": {
            "system": platform.system(),
            "release": platform.release(),
            "architecture": platform.machine(),
        },
        "revision": revision or None,
        "checks": checks,
        "state": "passed"
        if not any(
            check["state"] == "failed" and check["required"] for check in checks
        )
        else "failed",
    }
    if args.json_path:
        args.json_path.parent.mkdir(parents=True, exist_ok=True)
        args.json_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")

    for check in checks:
        print(f"[{check['state']}] {check['name']}: {check['detail']}")
    print(f"bootstrap {report['state']}")
    return 0 if report["state"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
