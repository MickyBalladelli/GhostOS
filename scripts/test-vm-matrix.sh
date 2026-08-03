#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root_dir"

echo "fast VM matrix"
cargo test -p synos-vm --lib
cargo test -p synos-vm --bin synos-vm
cargo test -p synos-vm --test matrix_59_11
cargo test -p synos-vm --test test_environments

if [[ "${SYNOS_RUN_QEMU_TESTS:-}" == "1" ]]; then
    echo "opt-in QEMU matrix"
    cargo test -p synos-vm --test qemu_matrix_59_11 -- --ignored
    cargo test -p synos-vm --test test_environments -- --ignored
else
    echo "QEMU matrix skipped; set SYNOS_RUN_QEMU_TESTS=1"
fi
