#!/usr/bin/env bash
set -Eeuo pipefail

target=${1:?usage: replay-vm-fuzz.sh TARGET RETAINED_INPUT}
input=${2:?usage: replay-vm-fuzz.sh TARGET RETAINED_INPUT}
root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

case "$target" in
    vm-decoder|vm-devices|vm-images|vm-snapshot|vm-terminal|vm-migration) ;;
    *) echo "unknown VM fuzz target: $target" >&2; exit 2 ;;
esac

case_path=$root_dir/$input
if [[ ! -f "$case_path" ]]; then
    echo "retained fuzz input does not exist: $input" >&2
    exit 2
fi
if ! command -v cargo-fuzz >/dev/null 2>&1; then
    echo "cargo-fuzz is required to replay VM fuzz inputs" >&2
    exit 2
fi

cd "$root_dir/fuzz"
exec cargo fuzz run "$target" "$case_path" --sanitizer none -- -runs=1
