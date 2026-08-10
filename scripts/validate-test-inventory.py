#!/usr/bin/env python3
"""Check that the TODO roadmap and test inventory still have matching IDs."""

import re
import subprocess
import sys
import tomllib
from pathlib import Path


root = Path(__file__).resolve().parent.parent
inventory_path = root / "docs/test-inventory.toml"
inventory_data = tomllib.loads(inventory_path.read_text())
todo = (root / str(inventory_data["source"])).read_text()
inventory = inventory_path.read_text()

todo_ids = sorted(
    {
        f"{int(value):02d}"
        for value in re.findall(r"^## (\d+)\.", todo, re.MULTILINE)
        if int(value) <= 58
    }
)
feature_ids = sorted(set(re.findall(r'^id = "(\d+)"$', inventory, re.MULTILINE)))
expected = [f"{number:02d}" for number in range(1, 59)]
errors = []

if todo_ids != expected:
    errors.append(f"TODO feature headings are not 01..58: {todo_ids}")
if feature_ids != expected:
    errors.append(f"inventory feature IDs are not 01..58: {feature_ids}")
if '"virtual_machine"' not in (root / "Cargo.toml").read_text():
    errors.append("virtual_machine is missing from the root workspace")
if '"crates/test-support"' not in (root / "Cargo.toml").read_text():
    errors.append("test-support is missing from root default-members")
for tier in ("unit", "integration", "qemu", "fault", "fuzz", "performance"):
    if f"[tiers.{tier}]" not in inventory:
        errors.append(f"missing inventory tier: {tier}")

if errors:
    for error in errors:
        print(f"inventory error: {error}", file=sys.stderr)
    sys.exit(1)

print(f"test inventory valid: {len(feature_ids)} roadmap features, six tiers")
coverage_check = subprocess.run(
    [sys.executable, str(root / "scripts" / "validate-test-coverage.py")],
    cwd=root,
    check=False,
)
if coverage_check.returncode:
    sys.exit(coverage_check.returncode)
