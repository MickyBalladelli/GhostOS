#!/usr/bin/env bash
set -Eeuo pipefail

evidence_dir=${1:?usage: release-gate.sh EVIDENCE_DIRECTORY}
root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

if [[ ! -d "$evidence_dir" ]]; then
    echo "release gate: evidence directory does not exist: $evidence_dir" >&2
    exit 1
fi

python3 - "$evidence_dir" <<'PY'
import json
import pathlib
import sys

evidence = pathlib.Path(sys.argv[1])
results = sorted(evidence.rglob("result.json"))
if not results:
    raise SystemExit("release gate: no result.json files")

failures = []
for path in results:
    try:
        result = json.loads(path.read_text())
    except json.JSONDecodeError as error:
        failures.append(f"{path}: invalid JSON ({error})")
        continue
    state = result.get("state")
    reason = result.get("reason")
    if state not in {"passed", "failed", "skipped"}:
        failures.append(f"{path}: invalid state {state!r}")
    elif not isinstance(reason, str) or not reason.strip():
        failures.append(f"{path}: result has no reason")
    elif state != "passed":
        failures.append(f"{path}: {state!r} ({reason})")

if failures:
    print("release gate failed:")
    print("\n".join(f"- {failure}" for failure in failures))
    raise SystemExit(1)

print(f"release gate passed: {len(results)} evidence results are passed")
PY

if rg -l "KERNEL PANIC|guest panic" "$evidence_dir" --glob '*.log' >/dev/null 2>&1; then
    echo "release gate: panic marker found in evidence" >&2
    exit 1
fi

if [[ -f "$evidence_dir/qemu/result.json" ]] && rg -q '"state"[[:space:]]*:[[:space:]]*"passed"' "$evidence_dir/qemu/result.json"; then
    if ! rg -q "SynOS kernel bootstrap" "$evidence_dir/qemu" --glob '*.log'; then
        echo "release gate: clean QEMU boot marker is missing" >&2
        exit 1
    fi
fi

python3 "$root_dir/scripts/validate-changelog.py"

revision=$(git -C "$root_dir" rev-parse HEAD)
if [[ -f "$evidence_dir/revision.txt" ]] && [[ "$(<"$evidence_dir/revision.txt")" != "$revision" ]]; then
    echo "release gate: evidence revision does not match HEAD" >&2
    exit 1
fi

for image in "$root_dir/build/bios/synos-bios.img" "$root_dir/build/portable/synos.img"; do
    if [[ -f "$image" ]]; then
        if [[ ! -f "$image.revision" || "$(<"$image.revision")" != "$revision" ]]; then
            echo "release gate: image provenance does not match HEAD: $image" >&2
            exit 1
        fi
    fi
done

echo "release gate passed for $revision"
