#!/usr/bin/env python3
"""Create a compact status dashboard from inventory and evidence results."""

import json
import pathlib
import sys
from collections import Counter


root = pathlib.Path(__file__).resolve().parent.parent
evidence = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else root / "build/test-evidence"
results = []
for path in sorted(evidence.rglob("result.json")) if evidence.exists() else []:
    try:
        result = json.loads(path.read_text())
    except json.JSONDecodeError:
        continue
    results.append((path, result))

counts = Counter(result.get("state", "unknown") for _, result in results)
dashboard = root / "build/test-dashboard.md"
dashboard.parent.mkdir(parents=True, exist_ok=True)
lines = [
    "# SynOS test status",
    "",
    f"Evidence root: `{evidence}`",
    "",
    "| State | Count |",
    "| --- | ---: |",
]
lines.extend(f"| {state} | {counts[state]} |" for state in sorted(counts))
lines += ["", "| Tier | State | Evidence |", "| --- | --- | --- |"]
for path, result in results:
    lines.append(f"| {result.get('tier', path.parent.name)} | {result.get('state')} | `{path}` |")
dashboard.write_text("\n".join(lines) + "\n")
print(dashboard)
