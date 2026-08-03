#!/usr/bin/env bash
set -Eeuo pipefail

root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root_dir"

if ! command -v cargo-mutants >/dev/null 2>&1; then
    echo "cargo-mutants is required for mutation validation" >&2
    exit 2
fi

packages=(synos-fsd synos-status synos-auth synos-synfs synos-http synos-vm)
if [[ -n "${SYNOS_MUTATION_PACKAGE:-}" ]]; then
    packages=("$SYNOS_MUTATION_PACKAGE")
fi
for package in "${packages[@]}"; do
    echo "mutation validation: $package"
    cargo mutants -p "$package" --timeout 60 --minimum-test-timeout 5
done
