#!/usr/bin/env python3
"""Validate the VM-specific quality-gate contract."""

from __future__ import annotations

import argparse
import os
import pathlib
import subprocess
import sys
import tomllib


ROOT = pathlib.Path(__file__).resolve().parent.parent
INVENTORY_PATH = ROOT / "virtual_machine/tests/inventory.toml"
REQUIRED_DEVICE_SCENARIOS = {
    "register_configuration",
    "normal_io",
    "reset",
    "interrupt",
    "malformed_input",
    "failure",
}
REQUIRED_BOOT_PATHS = {"bios", "uefi", "multiboot"}


def fail(errors: list[str], message: str) -> None:
    errors.append(message)


def check_file(errors: list[str], relative: str) -> pathlib.Path | None:
    path = ROOT / relative
    if not path.is_file():
        fail(errors, f"missing file: {relative}")
        return None
    return path


def validate_inventory(errors: list[str]) -> None:
    try:
        inventory = tomllib.loads(INVENTORY_PATH.read_text())
    except (OSError, tomllib.TOMLDecodeError) as error:
        fail(errors, f"cannot load VM inventory: {error}")
        return

    tests = {entry.get("id") for entry in inventory.get("test", [])}
    sources = {
        entry.get("path"): entry
        for entry in inventory.get("source", [])
        if entry.get("path")
    }
    public_apis = inventory.get("public_api", [])

    expected_sources = {
        path.relative_to(ROOT / "virtual_machine").as_posix()
        for path in (ROOT / "virtual_machine/src").rglob("*.rs")
    }
    missing_sources = sorted(expected_sources - set(sources))
    extra_sources = sorted(
        path for path in set(sources) - expected_sources if not path.startswith("tests/")
    )
    for path in missing_sources:
        fail(errors, f"VM source module has no inventory entry: {path}")
    for path in extra_sources:
        fail(errors, f"VM inventory names a non-source module: {path}")

    for path, entry in sorted(sources.items()):
        if not entry.get("tests"):
            fail(errors, f"VM source module has no named test: {path}")
        for test_id in entry.get("tests", []):
            if test_id not in tests:
                fail(errors, f"source {path} references unknown test ID: {test_id}")

    api_sources = {entry.get("source") for entry in public_apis}
    for path in sorted(expected_sources):
        source_text = (ROOT / "virtual_machine" / path).read_text(errors="ignore")
        has_public_surface = "pub " in source_text or "pub(" in source_text
        if has_public_surface and path not in api_sources:
            fail(errors, f"public VM API surface has no inventory entry: {path}")
    for entry in public_apis:
        path = entry.get("source")
        if path not in expected_sources:
            fail(errors, f"public API names unknown source: {path}")
        if not entry.get("name"):
            fail(errors, f"public API entry has no name: {path}")
        if not entry.get("tests"):
            fail(errors, f"public API has no named test: {entry.get('name')}")
        for test_id in entry.get("tests", []):
            if test_id not in tests:
                fail(errors, f"public API {entry.get('name')} references unknown test ID: {test_id}")

    devices = inventory.get("device", [])
    if not devices:
        fail(errors, "VM inventory has no device coverage matrix")
    for device in devices:
        name = device.get("name", "<unnamed>")
        source = device.get("source")
        if source not in expected_sources:
            fail(errors, f"device {name} names unknown source: {source}")
        scenarios = set(device.get("scenarios", []))
        if scenarios != REQUIRED_DEVICE_SCENARIOS:
            fail(errors, f"device {name} must declare all six required scenarios")
        scenario_tests = device.get("tests", [])
        if len(scenario_tests) != len(REQUIRED_DEVICE_SCENARIOS):
            fail(errors, f"device {name} must name one test for each required scenario")
        for test_id in scenario_tests:
            if test_id not in tests:
                fail(errors, f"device {name} references unknown test ID: {test_id}")

    boot_paths = {entry.get("name"): entry for entry in inventory.get("boot_path", [])}
    if set(boot_paths) != REQUIRED_BOOT_PATHS:
        fail(errors, f"VM boot inventory must contain exactly: {sorted(REQUIRED_BOOT_PATHS)}")
    for name, entry in sorted(boot_paths.items()):
        test_id = entry.get("test")
        if test_id not in tests:
            fail(errors, f"boot path {name} references unknown test ID: {test_id}")
        if not entry.get("serial_marker"):
            fail(errors, f"boot path {name} has no serial evidence marker")

    for entry in inventory.get("test", []):
        file = entry.get("file")
        if not file or not (ROOT / "virtual_machine" / file).is_file():
            fail(errors, f"test {entry.get('id')} has no test file: {file}")


def validate_commands(errors: list[str]) -> None:
    root_cargo = (ROOT / "Cargo.toml").read_text()
    if '"virtual_machine"' not in root_cargo:
        fail(errors, "root Cargo workspace does not include virtual_machine")
    if "./scripts/test-all.sh" not in (ROOT / "docs/testing.md").read_text():
        fail(errors, "docs/testing.md does not name the deterministic VM runner")
    if "SYNOS_FULL_VALIDATION=1 ./scripts/full-validation.sh" not in (ROOT / "docs/testing.md").read_text():
        fail(errors, "docs/testing.md does not name the full-validation runner")
    for script in ("scripts/test-all.sh", "scripts/full-validation.sh", "scripts/vm-soak.sh"):
        path = check_file(errors, script)
        if path and "set -Eeuo pipefail" not in path.read_text():
            fail(errors, f"{script} is not strict-mode")
    mutation = (ROOT / "scripts/mutation.sh").read_text()
    if "synos-vm" not in mutation or "cargo mutants" not in mutation:
        fail(errors, "VM mutation testing is not wired")
    fuzz_cargo = (ROOT / "fuzz/Cargo.toml").read_text()
    for target in ("vm-decoder", "vm-devices", "vm-images"):
        if f'name = "{target}"' not in fuzz_cargo:
            fail(errors, f"missing VM fuzz target: {target}")


def validate_ci(errors: list[str]) -> None:
    workflows = list((ROOT / ".github/workflows").glob("*.yml")) + list(
        (ROOT / ".github/workflows").glob("*.yaml")
    )
    if not workflows:
        fail(errors, "no GitHub workflow provides VM CI")
        return
    text = "\n".join(path.read_text() for path in workflows)
    for platform in ("ubuntu-latest", "macos-latest", "windows-latest"):
        if platform not in text:
            fail(errors, f"cross-platform CI is missing {platform}")
    for required in ("cargo test", "synos-vm", "schedule:"):
        if required not in text:
            fail(errors, f"CI is missing {required}")


def validate_changed_source(errors: list[str]) -> None:
    working_tree = subprocess.run(
        ["git", "diff", "--name-only", "HEAD"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if working_tree.returncode:
        fail(errors, f"cannot inspect changed VM sources: {working_tree.stderr.strip()}")
        return

    all_changed = working_tree.stdout.splitlines()
    changed_sources = [
        line for line in all_changed if line.startswith("virtual_machine/src/") and line.endswith(".rs")
    ]
    if not changed_sources:
        base = os.environ.get("VM_QUALITY_BASE")
        if not base and os.environ.get("GITHUB_BASE_REF"):
            base = f"origin/{os.environ['GITHUB_BASE_REF']}"
        base = base or "HEAD^"
        committed = subprocess.run(
            ["git", "diff", "--name-only", f"{base}...HEAD"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        if committed.returncode:
            fail(errors, f"cannot inspect committed VM sources: {committed.stderr.strip()}")
            return
        all_changed = committed.stdout.splitlines()
        changed_sources = [
            line for line in all_changed if line.startswith("virtual_machine/src/") and line.endswith(".rs")
        ]
    if changed_sources and not any(line.startswith("virtual_machine/tests/") for line in all_changed):
        fail(errors, "changed VM source without a VM regression/integration test change")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--changed",
        action="store_true",
        help="require a VM test change when VM production source changed",
    )
    args = parser.parse_args()
    errors: list[str] = []
    validate_inventory(errors)
    validate_commands(errors)
    validate_ci(errors)
    if args.changed:
        validate_changed_source(errors)
    if errors:
        print("VM quality gate failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print("VM quality gate passed: workspace, inventory, devices, boot, fuzz, and CI")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
