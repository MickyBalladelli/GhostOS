#!/usr/bin/env bash
set -Eeuo pipefail

root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root_dir"

runs=${SYNOS_VM_SOAK_RUNS:-3}
if ! [[ "$runs" =~ ^[1-9][0-9]*$ ]]; then
    echo "SYNOS_VM_SOAK_RUNS must be a positive integer" >&2
    exit 2
fi

for ((run = 1; run <= runs; run++)); do
    echo "VM soak run $run/$runs"
    cargo test -p synos-vm --all-targets
done

echo "VM soak passed: $runs bounded deterministic runs"
