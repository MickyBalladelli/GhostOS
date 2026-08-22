#!/usr/bin/env bash
set -Eeuo pipefail

root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root_dir/fuzz"

python3 "$root_dir/scripts/validate-fuzz-inventory.py"

if ! command -v cargo-fuzz >/dev/null 2>&1; then
    echo "cargo-fuzz is required for fuzz smoke validation" >&2
    exit 2
fi

runs=${GHOSTOS_FUZZ_RUNS:-1000}
timeout_seconds=${GHOSTOS_FUZZ_TIMEOUT_SECONDS:-10}
rss_limit_mb=${GHOSTOS_FUZZ_RSS_LIMIT_MB:-1024}
for target in path volume operations mount http script network manifest abi-frame cli vm-decoder vm-devices vm-images vm-snapshot vm-terminal vm-migration; do
    echo "fuzz smoke: $target ($runs runs)"
    set +e
    cargo fuzz run "$target" --sanitizer none -- -runs="$runs" -max_len=4096 -timeout="$timeout_seconds" -rss_limit_mb="$rss_limit_mb"
    status=$?
    set -e
    for artifact_kind in crash hang timeout oom leak excessive-allocation; do
        for artifact in "$root_dir/fuzz/artifacts/$target/$artifact_kind"-*; do
            [[ -f "$artifact" ]] || continue
            python3 "$root_dir/scripts/retain-fuzz-artifact.py" "$target" "$artifact"
        done
    done
    [[ $status -eq 0 ]] || exit "$status"
done
