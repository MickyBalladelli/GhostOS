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

state_aliases = {"pass": "passed", "fail": "failed"}
for _, result in results:
    state = result.get("state", "unknown")
    result["state"] = state_aliases.get(state, state)
counts = Counter(result["state"] for _, result in results)
dashboard = root / "build/test-dashboard.md"
dashboard.parent.mkdir(parents=True, exist_ok=True)
lines = [
    "# GhostOS test status",
    "",
    f"Evidence root: `{evidence}`",
    "",
    "| State | Count |",
    "| --- | ---: |",
]
lines.extend(f"| {state} | {counts[state]} |" for state in sorted(counts))
grouped = {}
for path, result in results:
    tier = str(result.get("tier") or path.relative_to(evidence).parts[0])
    grouped.setdefault(tier, []).append((path, result))
for tier in sorted(grouped):
    lines += ["", f"## {tier}", "", "| State | Reason | Evidence |", "| --- | --- | --- |"]
    for path, result in grouped[tier]:
        reason = str(result.get("reason", "missing reason")).replace("|", "\\|")
        lines.append(f"| {result.get('state')} | {reason} | `{path}` |")
dashboard.write_text("\n".join(lines) + "\n")
print(dashboard)
