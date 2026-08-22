#!/usr/bin/env bash
set -Eeuo pipefail

root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root_dir"

revision=$(git rev-parse HEAD)
if [[ -n "$(git status --porcelain)" ]]; then
    echo "reproducibility check requires a clean source tree" >&2
    exit 1
fi

project_root=$root_dir
. "$root_dir/scripts/reproducible-env.sh"
./scripts/build-bios-image.sh >/dev/null
first=$(mktemp /tmp/ghostos-bios-first.XXXXXX)
second=$(mktemp /tmp/ghostos-bios-second.XXXXXX)
cp build/bios/ghostos-bios.img "$first"
./scripts/build-bios-image.sh >/dev/null
cp build/bios/ghostos-bios.img "$second"
cmp "$first" "$second"

./scripts/build-uefi-loader.sh
loader_first=$(mktemp /tmp/ghostos-uefi-loader-first.XXXXXX)
cp target/x86_64-unknown-uefi/release/ghostos-loader.efi "$loader_first"
./scripts/build-uefi-loader.sh
cmp "$loader_first" target/x86_64-unknown-uefi/release/ghostos-loader.efi
./scripts/build-portable-image.sh >/dev/null
portable_first=$(mktemp /tmp/ghostos-portable-first.XXXXXX)
portable_second=$(mktemp /tmp/ghostos-portable-second.XXXXXX)
cp build/portable/ghostos.img "$portable_first"
./scripts/build-portable-image.sh >/dev/null
cp build/portable/ghostos.img "$portable_second"
cmp "$portable_first" "$portable_second"

printf '%s\n' "$revision" > build/bios/ghostos-bios.img.revision
printf '%s\n' "$revision" > build/portable/ghostos.img.revision
printf 'reproducible release images: revision=%s SOURCE_DATE_EPOCH=%s\n' "$revision" "$SOURCE_DATE_EPOCH"
