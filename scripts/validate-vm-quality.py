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
GENERATED_INVENTORY_PATH = ROOT / "virtual_machine/tests/generated-inventory.toml"
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
    generation_check = subprocess.run(
        [sys.executable, str(ROOT / "scripts/generate-vm-inventory.py"), "--check"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if generation_check.returncode:
        fail(errors, generation_check.stderr.strip() or "VM generated inventory check failed")

    try:
        inventory = tomllib.loads(INVENTORY_PATH.read_text())
        generated = tomllib.loads(GENERATED_INVENTORY_PATH.read_text())
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

    expected_sources = {entry.get("path") for entry in generated.get("source", [])}
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

    for source_entry in generated.get("source", []):
        source = source_entry.get("path")
        for entry in source_entry.get("public_api", []):
            for name in entry.get("symbols", []):
                if source not in api_sources:
                    fail(errors, f"generated public API has no coverage entry: {source}::{name}")
                if not entry.get("tests"):
                    fail(errors, f"generated public API has no named test: {source}::{name}")
                for test_id in entry.get("tests", []):
                    if test_id not in tests:
                        fail(errors, f"generated public API {source}::{name} references unknown test ID: {test_id}")

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

    device_sources = {entry.get("source") for entry in devices}
    for source_entry in generated.get("source", []):
        source = source_entry.get("path")
        for entry in source_entry.get("device", []):
            for name in entry.get("symbols", []):
                if source not in device_sources:
                    fail(errors, f"generated device has no coverage entry: {source}::{name}")
                if not entry.get("tests"):
                    fail(errors, f"generated device has no named test: {source}::{name}")
                for test_id in entry.get("tests", []):
                    if test_id not in tests:
                        fail(errors, f"generated device {source}::{name} references unknown test ID: {test_id}")

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
    if "GHOSTOS_FULL_VALIDATION=1 ./scripts/full-validation.sh" not in (ROOT / "docs/testing.md").read_text():
        fail(errors, "docs/testing.md does not name the full-validation runner")
    if "GHOSTOS_SOAK_RUNS=3 ./scripts/soak.sh" not in (ROOT / "docs/testing.md").read_text():
        fail(errors, "docs/testing.md does not name the workflow soak runner")
    for script in (
        "scripts/test-all.sh",
        "scripts/full-validation.sh",
        "scripts/vm-soak.sh",
        "scripts/soak.sh",
    ):
        path = check_file(errors, script)
        if path and "set -Eeuo pipefail" not in path.read_text():
            fail(errors, f"{script} is not strict-mode")
    for script in (
        "scripts/vm-soak.py",
        "scripts/soak.py",
        "scripts/record-vm-evidence.py",
        "scripts/validate-vm-evidence.py",
    ):
        check_file(errors, script)
    test_runner = (ROOT / "scripts/test-all.sh").read_text()
    for field in ("--command", "--firmware", "--cpu-count", "--result-file"):
        if field not in test_runner:
            fail(errors, f"VM runner does not record {field}")
    mutation = (ROOT / "scripts/mutation.sh").read_text()
    if "ghostos-vm" not in mutation or "cargo mutants" not in mutation:
        fail(errors, "VM mutation testing is not wired")
    fuzz_cargo = (ROOT / "fuzz/Cargo.toml").read_text()
    for target in (
        "vm-decoder",
        "vm-devices",
        "vm-images",
        "vm-snapshot",
        "vm-terminal",
        "vm-migration",
    ):
        if f'name = "{target}"' not in fuzz_cargo:
            fail(errors, f"missing VM fuzz target: {target}")
        if not (ROOT / "fuzz/corpus" / target).is_dir():
            fail(errors, f"missing retained corpus directory for {target}")
    for script in ("scripts/retain-vm-fuzz-crash.py", "scripts/replay-vm-fuzz.sh"):
        check_file(errors, script)


def validate_local_quality(errors: list[str]) -> None:
    for script in (
        "scripts/test-all.sh",
        "scripts/full-validation.sh",
        "scripts/validate-vm-quality.py",
        "scripts/check-vm-platform.py",
        "docs/host-portability-matrix.toml",
    ):
        check_file(errors, script)
    platform_probe = ROOT / "scripts/check-vm-platform.py"
    if platform_probe.is_file() and '"state": "skipped"' not in platform_probe.read_text():
        fail(errors, "platform capability probe does not record explicit skips")


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
    validate_local_quality(errors)
    if args.changed:
        validate_changed_source(errors)
    if errors:
        print("VM quality gate failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print("VM quality gate passed: workspace, inventory, devices, boot, fuzz, and local validation")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
