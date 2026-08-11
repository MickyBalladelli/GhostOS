#!/usr/bin/env python3
"""Validate that every fuzz target has an owner and a retained seed corpus."""

from __future__ import annotations

import pathlib
import sys
import tomllib


ROOT = pathlib.Path(__file__).resolve().parent.parent
FUZZ = ROOT / "fuzz"


def main() -> int:
    data = tomllib.loads((FUZZ / "triage.toml").read_text())
    targets = {entry["name"]: entry for entry in data["target"]}
    binaries = {
        entry["name"]
        for entry in tomllib.loads((FUZZ / "Cargo.toml").read_text()).get("bin", [])
    }
    errors = []
    if binaries != set(targets):
        errors.append(f"Cargo targets and triage targets differ: {binaries ^ set(targets)}")
    for name, target in targets.items():
        corpus = FUZZ / "corpus" / name
        seeds = [path for path in corpus.iterdir() if path.name != ".gitkeep"] if corpus.is_dir() else []
        if not target.get("owner"):
            errors.append(f"{name}: missing owner")
        if not target.get("test_id"):
            errors.append(f"{name}: missing test_id")
        if not seeds:
            errors.append(f"{name}: corpus has no retained seed")
    if errors:
        for error in errors:
            print(f"fuzz inventory error: {error}", file=sys.stderr)
        return 1
    print(f"fuzz inventory valid: {len(targets)} targets, assigned owners, retained seeds")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
