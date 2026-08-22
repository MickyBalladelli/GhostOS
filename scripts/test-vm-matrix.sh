#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root_dir"

python3 scripts/check-vm-platform.py \
    --output "${GHOSTOS_PORTABILITY_REPORT:-build/test-evidence/host-portability.json}"

echo "fast VM matrix"
cargo test -p ghostos-vm --lib
cargo test -p ghostos-vm --bin ghostos-vm
cargo test -p ghostos-vm --test matrix_59_11
cargo test -p ghostos-vm --test test_environments

if [[ "${GHOSTOS_RUN_QEMU_TESTS:-}" == "1" ]]; then
    echo "opt-in QEMU matrix"
    cargo test -p ghostos-vm --test qemu_matrix_59_11 --test test_environments --test qemu_login_e2e -- --ignored
else
    echo "QEMU matrix skipped; set GHOSTOS_RUN_QEMU_TESTS=1"
fi
