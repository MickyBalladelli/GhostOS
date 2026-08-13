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
first=$(mktemp /tmp/synos-bios-first.XXXXXX)
second=$(mktemp /tmp/synos-bios-second.XXXXXX)
cp build/bios/synos-bios.img "$first"
./scripts/build-bios-image.sh >/dev/null
cp build/bios/synos-bios.img "$second"
cmp "$first" "$second"

./scripts/build-uefi-loader.sh
loader_first=$(mktemp /tmp/synos-uefi-loader-first.XXXXXX)
cp target/x86_64-unknown-uefi/release/synos-loader.efi "$loader_first"
./scripts/build-uefi-loader.sh
cmp "$loader_first" target/x86_64-unknown-uefi/release/synos-loader.efi
./scripts/build-portable-image.sh >/dev/null
portable_first=$(mktemp /tmp/synos-portable-first.XXXXXX)
portable_second=$(mktemp /tmp/synos-portable-second.XXXXXX)
cp build/portable/synos.img "$portable_first"
./scripts/build-portable-image.sh >/dev/null
cp build/portable/synos.img "$portable_second"
cmp "$portable_first" "$portable_second"

printf '%s\n' "$revision" > build/bios/synos-bios.img.revision
printf '%s\n' "$revision" > build/portable/synos.img.revision
printf 'reproducible release images: revision=%s SOURCE_DATE_EPOCH=%s\n' "$revision" "$SOURCE_DATE_EPOCH"
