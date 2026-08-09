#!/usr/bin/env bash
set -Eeuo pipefail

root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root_dir"

run_id=${SYNOS_TEST_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)-$$}
evidence_dir=${SYNOS_EVIDENCE_DIR:-$root_dir/build/test-evidence/$run_id}
mkdir -p "$evidence_dir"

write_metadata() {
    local tier=$1
    local command=$2
    local output_dir=$3
    local revision
    local started_at=$4
    local ended_at=$5
    revision=$(git rev-parse HEAD 2>/dev/null || printf 'unknown')
    python3 - "$tier" "$command" "$revision" "$started_at" "$ended_at" "$output_dir" <<'PY'
import json
import os
import pathlib
import platform
import subprocess
import sys

environment = {}
for key, value in os.environ.items():
    if key.startswith("SYNOS_"):
        environment[key] = "<redacted>" if any(token in key.lower() for token in ("token", "secret", "password")) else value

metadata = {
    "tier": sys.argv[1],
    "command": sys.argv[2],
    "revision": sys.argv[3],
    "started_at": sys.argv[4],
    "ended_at": sys.argv[5],
    "host": platform.system(),
    "arch": platform.machine(),
    "rust": subprocess.run(["rustc", "--version"], capture_output=True, text=True, check=False).stdout.strip(),
    "environment": environment,
}
pathlib.Path(sys.argv[6], "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
PY
}

run_tier() {
    local tier=$1
    shift
    local output_dir="$evidence_dir/$tier"
    local command="$*"
    local started_at
    local ended_at
    mkdir -p "$output_dir"
    started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)

    echo "== $tier: $command"
    set +e
    "$@" > >(tee "$output_dir/stdout.log") 2> >(tee "$output_dir/stderr.log" >&2)
    local status=$?
    set -e
    ended_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    write_metadata "$tier" "$command" "$output_dir" "$started_at" "$ended_at"
    if [[ $status -eq 0 ]]; then
        printf '{"schema":1,"state":"passed","tier":"%s","reason":"command completed successfully"}\n' "$tier" > "$output_dir/result.json"
    else
        printf '{"schema":1,"state":"failed","tier":"%s","exit_code":%d,"reason":"command exited with status %d"}\n' "$tier" "$status" "$status" > "$output_dir/result.json"
    fi
    if [[ "$tier" == vm ]]; then
        for inventory_tier in fast-unit vm-integration cli; do
            python3 "$root_dir/scripts/record-vm-evidence.py" \
                --evidence-dir "$evidence_dir" \
                --tier "$inventory_tier" \
                --command "$command" \
                --firmware "bios,uefi,multiboot" \
                --cpu-count "${SYNOS_VM_CPUS:-2}" \
                --result-file "$output_dir/result.json" \
                --started-at "$started_at" \
                --ended-at "$ended_at"
        done
    fi
    [[ $status -eq 0 ]] || return "$status"
}

run_tier host-unit cargo test
run_tier unit cargo test -p synos-vm --lib
run_tier integration cargo test -p synos-vm --tests
run_tier workspace cargo test --workspace --all-targets
run_tier vm cargo test -p synos-vm --all-targets
run_tier vm-quality python3 "$root_dir/scripts/validate-vm-quality.py"
benchmark_revision=$(git rev-parse HEAD 2>/dev/null || printf unknown)
run_tier performance env SYNOS_BENCH_REVISION="$benchmark_revision" cargo bench -p synos-vm --bench bounded
run_tier recovery cargo test --workspace --all-targets

python3 "$root_dir/scripts/validate-vm-evidence.py" "$evidence_dir" \
    --require-tier fast-unit \
    --require-tier vm-integration \
    --require-tier cli

printf '%s\n' "$(git rev-parse HEAD 2>/dev/null || printf unknown)" > "$evidence_dir/revision.txt"
printf '%s\n' "$evidence_dir" > "$evidence_dir/run-path.txt"
echo "deterministic validation passed; evidence: $evidence_dir"
