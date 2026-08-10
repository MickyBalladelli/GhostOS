#!/usr/bin/env bash
set -Eeuo pipefail

root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root_dir/fuzz"

if ! command -v cargo-fuzz >/dev/null 2>&1; then
    echo "cargo-fuzz is required for fuzz smoke validation" >&2
    exit 2
fi

runs=${SYNOS_FUZZ_RUNS:-1000}
for target in path volume operations mount http script vm-decoder vm-devices vm-images vm-snapshot vm-terminal vm-migration network; do
    echo "fuzz smoke: $target ($runs runs)"
    set +e
    cargo fuzz run "$target" --sanitizer none -- -runs="$runs" -max_len=4096
    status=$?
    set -e
    for artifact_kind in crash timeout oom leak; do
        for artifact in "$root_dir/fuzz/artifacts/$target/$artifact_kind"-*; do
            [[ -f "$artifact" ]] || continue
            python3 "$root_dir/scripts/retain-vm-fuzz-crash.py" "$target" "$artifact"
        done
    done
    [[ $status -eq 0 ]] || exit "$status"
done
