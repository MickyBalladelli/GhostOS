#!/usr/bin/env bash
set -Eeuo pipefail

root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root_dir/fuzz"

if ! command -v cargo-fuzz >/dev/null 2>&1; then
    echo "cargo-fuzz is required for fuzz smoke validation" >&2
    exit 2
fi

runs=${SYNOS_FUZZ_RUNS:-1000}
for target in path volume operations mount http script vm-decoder vm-devices vm-images; do
    echo "fuzz smoke: $target ($runs runs)"
    cargo fuzz run "$target" --sanitizer none -- -runs="$runs" -max_len=4096
done
