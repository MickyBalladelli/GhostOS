#!/usr/bin/env python3
"""Ensure external validation results stay in their own evidence tiers."""

from __future__ import annotations

import argparse
import json
import pathlib
import sys


SEPARATE_TIERS = ("qemu", "hardware-accelerated", "hardware-boot", "fuzz", "soak")
RESULT_NAMES = {"result.json", "evidence.json"}


def load_json(path: pathlib.Path, errors: list[str]) -> dict[str, object] | None:
    try:
        value = json.loads(path.read_text())
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        errors.append(f"{path}: invalid JSON ({error})")
        return None
    if not isinstance(value, dict):
        errors.append(f"{path}: JSON value must be an object")
        return None
    return value


def validate(evidence_dir: pathlib.Path, required_tiers: set[str]) -> list[str]:
    errors: list[str] = []
    for tier in required_tiers:
        result_path = evidence_dir / tier / "result.json"
        if not result_path.is_file():
            errors.append(f"missing isolated tier result: {result_path}")

    for path in sorted(evidence_dir.rglob("*")):
        if not path.is_file() or path.is_symlink() or path.name not in RESULT_NAMES:
            continue
        relative = path.relative_to(evidence_dir)
        section = relative.parts[0] if relative.parts else ""
        value = load_json(path, errors)
        if value is None:
            continue
        declared = value.get("tier")
        if isinstance(declared, str) and declared in SEPARATE_TIERS and section != declared:
            errors.append(
                f"{path}: tier {declared!r} is stored under {section!r}; "
                "external results must stay in their own directory"
            )
        if section not in SEPARATE_TIERS:
            continue
        if not isinstance(declared, str):
            if section != "hardware-boot" or path.name != "evidence.json":
                errors.append(f"{path}: isolated evidence must declare tier {section!r}")
            continue
        if declared != section:
            errors.append(f"{path}: tier must be {section!r}, got {declared!r}")
        if path.name == "result.json" and len(relative.parts) != 2:
            errors.append(f"{path}: isolated tier result must be directly under {section}/")

    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence_dir", type=pathlib.Path)
    parser.add_argument(
        "--require-tier",
        action="append",
        choices=SEPARATE_TIERS,
        default=[],
        help="require a tier result.json (may be repeated)",
    )
    args = parser.parse_args()
    evidence_dir = args.evidence_dir.expanduser().resolve()
    if not evidence_dir.is_dir():
        print(f"evidence directory does not exist: {evidence_dir}", file=sys.stderr)
        return 1
    errors = validate(evidence_dir, set(args.require_tier))
    if errors:
        print("evidence separation validation failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print("evidence separation validation passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
