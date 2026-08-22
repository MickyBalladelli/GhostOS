#!/usr/bin/env bash
set -Eeuo pipefail

root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root_dir"

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
    echo "cargo-llvm-cov is required for coverage validation" >&2
    exit 2
fi

coverage_dir=${GHOSTOS_COVERAGE_DIR:-$root_dir/build/coverage}
mkdir -p "$coverage_dir/crates"

cargo llvm-cov --workspace --all-targets --json --output-path "$coverage_dir/workspace.json"

mapfile -t workspace_packages < <(
    cargo metadata --no-deps --format-version 1 \
        | python3 -c 'import json, sys; print("\\n".join(package["name"] for package in json.load(sys.stdin)["packages"]))'
)
for package in "${workspace_packages[@]}"; do
    echo "coverage: $package"
    cargo llvm-cov --package "$package" --all-targets --json \
        --output-path "$coverage_dir/crates/$package.json"
done

python3 - "$root_dir" "$coverage_dir" <<'PY'
import json
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
directory = pathlib.Path(sys.argv[2])
policy = (root / "coverage.toml").read_text()
total_min = float(re.search(r"total_lines\s*=\s*([0-9.]+)", policy).group(1))
crate_min = float(re.search(r"crate_lines\s*=\s*([0-9.]+)", policy).group(1))

def line_percent(path):
    report = json.loads(path.read_text())
    totals = report.get("data", [{}])[0].get("totals", {})
    return float(totals.get("lines", {}).get("percent", 0.0))

workspace_percent = line_percent(directory / "workspace.json")
crate_reports = {}
for path in sorted((directory / "crates").glob("*.json")):
    crate_reports[path.stem] = line_percent(path)

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
        "line_threshold": crate_min,
        "tests": mapped,
    }
(directory / "feature-summary.json").write_text(json.dumps(feature_summary, indent=2) + "\n")

summary = {
    "workspace_lines": workspace_percent,
    "minimum_workspace_lines": total_min,
    "crates": crate_reports,
    "minimum_crate_lines": crate_min,
    "todo_features": feature_summary,
}
(directory / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")

if workspace_percent < total_min:
    raise SystemExit(f"workspace line coverage {workspace_percent:.2f}% is below {total_min:.2f}%")
under = {name: value for name, value in crate_reports.items() if value < crate_min}
if under:
    raise SystemExit(f"crate line coverage below threshold: {under}")
if len(feature_summary) != 58:
    raise SystemExit(f"expected 58 TODO feature mappings, found {len(feature_summary)}")
print(f"coverage passed: workspace {workspace_percent:.2f}%, {len(crate_reports)} crates, 58 TODO mappings")
PY
