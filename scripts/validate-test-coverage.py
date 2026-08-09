#!/usr/bin/env python3
"""Enforce the definition of done in TODO.md section 59.13."""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import subprocess
import sys
import tomllib


ROOT = pathlib.Path(__file__).resolve().parent.parent
INVENTORY_PATH = ROOT / "docs" / "test-inventory.toml"
POLICY_PATH = ROOT / "docs" / "test-coverage.toml"
TODO_PATH = ROOT / "TODO.md"


def error(errors: list[str], message: str) -> None:
    errors.append(message)


def load_workspace(errors: list[str]) -> dict:
    result = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode:
        error(errors, f"cargo metadata failed: {result.stderr.strip()}")
        return {"packages": []}
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        error(errors, f"cargo metadata returned invalid JSON: {exc}")
        return {"packages": []}


def source_has_tests(package_root: pathlib.Path) -> bool:
    files = list((package_root / "src").rglob("*.rs"))
    tests_dir = package_root / "tests"
    if tests_dir.is_dir():
        files.extend(tests_dir.rglob("*.rs"))
    test_pattern = re.compile(r"#\s*\[\s*test\s*\]")
    return any(test_pattern.search(path.read_text(errors="ignore")) for path in files)


def validate_crates(policy: dict, inventory: dict, errors: list[str]) -> None:
    exceptions = {
        entry["name"]: entry
        for entry in policy.get("crate", [])
        if entry.get("name")
    }
    workspace = load_workspace(errors)
    production_packages: dict[str, pathlib.Path] = {}

    for package in workspace.get("packages", []):
        package_root = pathlib.Path(package["manifest_path"]).parent
        targets = package.get("targets", [])
        if any(
            kind in {"lib", "bin", "cdylib", "staticlib", "proc-macro"}
            for target in targets
            for kind in target.get("kind", [])
        ):
            production_packages[package["name"]] = package_root

    for name, package_root in sorted(production_packages.items()):
        if source_has_tests(package_root):
            continue
        entry = exceptions.get(name)
        if entry is None:
            error(errors, f"{name} has production code but no direct test and no 59.13 exception")
            continue
        if not entry.get("reason", "").strip():
            error(errors, f"{name} exception has no reason")
        replacement = entry.get("replacement_integration", "")
        if not re.fullmatch(r"integration\.\d{2}\.[a-z0-9-]+", replacement):
            error(errors, f"{name} exception must name an integration test ID")
    unknown = sorted(set(exceptions) - set(production_packages))
    for name in unknown:
        error(errors, f"59.13 crate exception names unknown package {name}")
    integration_ids = {
        test_id
        for entry in inventory.get("feature", [])
        for test_id in entry.get("integration", [])
    }
    for name, entry in exceptions.items():
        replacement = entry.get("replacement_integration")
        if replacement and replacement not in integration_ids:
            error(errors, f"{name} replacement integration is not in test inventory: {replacement}")


def todo_features() -> dict[str, str]:
    text = TODO_PATH.read_text()
    matches = re.finditer(
        r"^##\s+(\d+)\.([^\n]*)\n(.*?)(?=^##\s+|\Z)",
        text,
        re.MULTILINE | re.DOTALL,
    )
    features = {}
    for match in matches:
        number = int(match.group(1))
        if number > 58:
            continue
        features[f"{number:02d}"] = match.group(3)
    return features


def validate_feature_evidence(inventory: dict, policy: dict, errors: list[str]) -> None:
    features = {
        str(entry.get("id", "")).zfill(2): entry
        for entry in inventory.get("feature", [])
        if entry.get("id")
    }
    marked_features = {
        feature_id: body
        for feature_id, body in todo_features().items()
        if re.search(r"^-\s*\[x\]", body, re.MULTILINE)
    }
    required = policy.get("policy", {}).get(
        "required_feature_evidence",
        ["unit", "boundary", "integration", "end_to_end"],
    )
    aliases = {
        "boundary": policy.get("policy", {}).get("boundary_tier", "integration"),
        "end_to_end": policy.get("policy", {}).get("end_to_end_tier", "qemu"),
    }

    for feature_id in sorted(marked_features):
        entry = features.get(feature_id)
        if entry is None:
            error(errors, f"marked TODO feature {feature_id} has no inventory entry")
            continue
        for evidence_kind in required:
            tier = aliases.get(evidence_kind, evidence_kind)
            mapped = entry.get(tier)
            if not isinstance(mapped, list) or not mapped:
                error(errors, f"feature {feature_id} has no {evidence_kind} evidence mapping")
                continue
            for test_id in mapped:
                if not isinstance(test_id, str) or not re.fullmatch(
                    rf"{re.escape(tier)}\.\d{{2}}\.[a-z0-9-]+", test_id
                ):
                    error(errors, f"feature {feature_id} has invalid {tier} test ID {test_id!r}")

        negative_tier = policy.get("policy", {}).get("negative_tier", "fault")
        negative = entry.get(negative_tier)
        if not isinstance(negative, list) or not negative:
            error(errors, f"feature {feature_id} has no negative/error-boundary test")

    inventory_ids = {
        test_id
        for entry in inventory.get("feature", [])
        for tier in ("unit", "integration", "qemu", "fault", "fuzz", "performance")
        for test_id in entry.get(tier, [])
        if isinstance(test_id, str)
    }
    for boot_path in policy.get("boot_path", []):
        test_id = boot_path.get("test_id")
        if test_id not in inventory_ids:
            error(errors, f"boot path {boot_path.get('name')} references unknown test ID {test_id!r}")


def validate_persistent_features(inventory: dict, policy: dict, errors: list[str]) -> None:
    features = {
        str(entry.get("id", "")).zfill(2): entry
        for entry in inventory.get("feature", [])
        if entry.get("id")
    }
    keywords = (
        "cluster",
        "storage",
        "filesystem",
        "volume",
        "recovery",
        "persistent",
        "runtime",
        "federat",
        "mesh",
        "cache",
        "package",
        "update",
        "replay",
        "backup",
        "rms",
        "daemon",
    )
    for feature_id, body in todo_features().items():
        lowered = body.lower()
        if not any(keyword in lowered for keyword in keywords):
            continue
        entry = features.get(feature_id)
        if entry is None:
            continue
        for tier in ("integration", "fault"):
            if not entry.get(tier):
                error(errors, f"stateful/distributed feature {feature_id} has no {tier} scenario bundle")
        fault_text = " ".join(entry.get("fault", []))
        if not fault_text.startswith("fault."):
            error(errors, f"stateful/distributed feature {feature_id} has no fault scenario bundle")


def validate_vm_and_boot(policy: dict, errors: list[str]) -> None:
    policy_data = policy.get("policy", {})
    scenarios = set(policy_data.get("vm_device_scenarios", []))
    expected_scenarios = {
        "register_configuration",
        "normal_io",
        "reset",
        "interrupt",
        "malformed_input",
        "failure",
    }
    if scenarios != expected_scenarios:
        error(errors, "VM device scenario policy must cover register/configuration, I/O, reset, interrupt, malformed input, and failure")

    for device in policy.get("vm_device", []):
        name = device.get("name", "<unnamed>")
        source = ROOT / device.get("source", "")
        if not source.is_file():
            error(errors, f"VM device {name} source is missing: {device.get('source')}")
        if not re.fullmatch(r"vm\.59\.13\.[a-z0-9-]+", device.get("test_id", "")):
            error(errors, f"VM device {name} has invalid stable test ID")
        if set(device.get("scenarios", [])) != expected_scenarios:
            error(errors, f"VM device {name} does not declare all six required scenarios")

    for boot_path in policy.get("boot_path", []):
        name = boot_path.get("name", "<unnamed>")
        if not boot_path.get("vm_test"):
            error(errors, f"boot path {name} has no VM test")
        if not boot_path.get("serial_marker"):
            error(errors, f"boot path {name} has no serial marker")


def validate_documented_runner(errors: list[str]) -> None:
    docs = (ROOT / "docs" / "testing.md").read_text()
    required_text = (
        "./scripts/test-all.sh",
        "SYNOS_FULL_VALIDATION=1 ./scripts/full-validation.sh",
        "deterministic",
        "bounded",
        "isolated",
        "serial",
        "restart",
        "corruption",
    )
    for text in required_text:
        if text not in docs:
            error(errors, f"docs/testing.md does not document {text!r}")
    for script_name in ("test-all.sh", "full-validation.sh", "release-gate.sh"):
        script = ROOT / "scripts" / script_name
        if "set -Eeuo pipefail" not in script.read_text():
            error(errors, f"{script_name} is not strict-mode and may hide a failed tier")


def read_result(path: pathlib.Path, errors: list[str]) -> dict | None:
    try:
        value = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        error(errors, f"{path}: invalid result JSON ({exc})")
        return None
    if not isinstance(value, dict):
        error(errors, f"{path}: result must be an object")
        return None
    return value


def validate_evidence(evidence_dir: pathlib.Path, policy: dict, errors: list[str]) -> dict:
    required_tiers = ("host-unit", "workspace", "vm", "recovery", "qemu")
    states = {}
    for tier in required_tiers:
        result_path = evidence_dir / tier / "result.json"
        if not result_path.is_file():
            error(errors, f"missing required evidence result: {result_path}")
            continue
        result = read_result(result_path, errors)
        if result is None:
            continue
        state = result.get("state")
        states[tier] = state
        if state != policy.get("policy", {}).get("passing_state", "passed"):
            error(errors, f"{result_path}: state is {state!r}, expected 'passed'")

    qemu_logs = [
        path.read_text(errors="ignore")
        for path in (evidence_dir / "qemu").rglob("*.log")
    ] if (evidence_dir / "qemu").is_dir() else []
    combined_qemu = "\n".join(qemu_logs)
    for boot_path in policy.get("boot_path", []):
        marker = boot_path.get("serial_marker", "")
        if marker and marker not in combined_qemu:
            error(errors, f"boot path {boot_path.get('name')} lacks serial evidence {marker!r}")

    report = {
        "section": "59.13",
        "state": "passed" if not errors else "failed",
        "tiers": states,
        "checked": {
            "persistent_scenarios": policy.get("policy", {}).get("persistent_scenarios", []),
            "vm_devices": len(policy.get("vm_device", [])),
            "boot_paths": len(policy.get("boot_path", [])),
        },
    }
    (evidence_dir / "definition-of-done.json").write_text(
        json.dumps(report, indent=2) + "\n"
    )
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "evidence_dir",
        nargs="?",
        type=pathlib.Path,
        help="full-validation evidence directory; omit for static validation",
    )
    args = parser.parse_args()

    errors: list[str] = []
    try:
        inventory = tomllib.loads(INVENTORY_PATH.read_text())
        policy = tomllib.loads(POLICY_PATH.read_text())
    except (OSError, tomllib.TOMLDecodeError) as exc:
        print(f"59.13 validation could not load inventory: {exc}", file=sys.stderr)
        return 1

    validate_crates(policy, inventory, errors)
    validate_feature_evidence(inventory, policy, errors)
    validate_persistent_features(inventory, policy, errors)
    validate_vm_and_boot(policy, errors)
    validate_documented_runner(errors)

    if args.evidence_dir is not None:
        validate_evidence(args.evidence_dir, policy, errors)

    if errors:
        print("59.13 coverage definition failed:")
        print("\n".join(f"- {item}" for item in errors))
        return 1

    print(
        "59.13 coverage definition passed: "
        f"{len(todo_features())} roadmap features, "
        f"{len(policy.get('vm_device', []))} VM device families, "
        f"{len(policy.get('boot_path', []))} boot paths"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
