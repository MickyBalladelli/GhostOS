#!/usr/bin/env bash
set -Eeuo pipefail

root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root_dir"

if ! command -v cargo-mutants >/dev/null 2>&1; then
    echo "cargo-mutants is required for mutation validation" >&2
    exit 2
fi

packages=(ghostos-fsd ghostos-status ghostos-auth ghostos-ghostfs ghostos-http ghostos-vm)
if [[ -n "${GHOSTOS_MUTATION_PACKAGE:-}" ]]; then
    packages=("$GHOSTOS_MUTATION_PACKAGE")
fi
for package in "${packages[@]}"; do
    echo "mutation validation: $package"
    cargo mutants -p "$package" --timeout 60 --minimum-test-timeout 5
done
