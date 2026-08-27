#!/usr/bin/env bash
set -Eeuo pipefail

root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root_dir"

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
    echo "cargo-llvm-cov is required for coverage validation" >&2
    exit 2
fi

coverage_dir=${GHOSTOS_COVERAGE_DIR:-$root_dir/build/coverage}
mkdir -p "$coverage_dir"

cargo llvm-cov --workspace --all-targets --json --output-path "$coverage_dir/workspace.json"

python3 - "$root_dir" "$coverage_dir" <<'PY'
import json
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
directory = pathlib.Path(sys.argv[2])
policy = (root / "coverage.toml").read_text()
total_min = float(re.search(r"total_lines\s*=\s*([0-9.]+)", policy).group(1))
feature_min = float(re.search(r"todo_feature_lines\s*=\s*([0-9.]+)", policy).group(1))

def line_percent(path):
    report = json.loads(path.read_text())
    totals = report.get("data", [{}])[0].get("totals", {})
    return float(totals.get("lines", {}).get("percent", 0.0))

workspace_percent = line_percent(directory / "workspace.json")

inventory = (root / "docs/test-inventory.toml").read_text()
blocks = re.findall(r'^\[\[feature\]\]\n(.*?)(?=^\[\[feature\]\]|\Z)', inventory, re.MULTILINE | re.DOTALL)
feature_summary = {}
for block in blocks:
    feature = re.search(r'^id = "([0-9]+)"$', block, re.MULTILINE).group(1)
    mapped = {}
    for tier in ("unit", "integration", "qemu", "fault", "fuzz", "performance"):
        match = re.search(rf'^{tier} = (.+)$', block, re.MULTILINE)
        if not match:
            raise SystemExit(f"TODO feature {feature} has no {tier} coverage mapping")
        mapped[tier] = match.group(1)
    feature_summary[feature] = {
        "inventory_entry": True,
        "line_threshold": feature_min,
        "tests": mapped,
    }
(directory / "feature-summary.json").write_text(json.dumps(feature_summary, indent=2) + "\n")

summary = {
    "workspace_lines": workspace_percent,
    "minimum_workspace_lines": total_min,
    "minimum_feature_lines": feature_min,
    "todo_features": feature_summary,
}
(directory / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")

if workspace_percent < total_min:
    raise SystemExit(f"workspace line coverage {workspace_percent:.2f}% is below {total_min:.2f}%")
if len(feature_summary) != 58:
    raise SystemExit(f"expected 58 TODO feature mappings, found {len(feature_summary)}")
print(f"coverage passed: workspace {workspace_percent:.2f}%, 58 TODO mappings")
PY
