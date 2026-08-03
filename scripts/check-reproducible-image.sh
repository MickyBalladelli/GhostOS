#!/usr/bin/env bash
set -Eeuo pipefail

root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root_dir"

revision=$(git rev-parse HEAD)
if [[ -n "$(git status --porcelain)" ]]; then
    echo "reproducibility check requires a clean source tree" >&2
    exit 1
fi

export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git show -s --format=%ct "$revision")}
./scripts/build-bios-image.sh >/dev/null
first=$(mktemp /tmp/synos-bios-first.XXXXXX)
second=$(mktemp /tmp/synos-bios-second.XXXXXX)
cp build/bios/synos-bios.img "$first"
./scripts/build-bios-image.sh >/dev/null
cp build/bios/synos-bios.img "$second"
cmp "$first" "$second"

printf '%s\n' "$revision" > build/bios/synos-bios.img.revision
printf 'reproducible BIOS image: revision=%s SOURCE_DATE_EPOCH=%s\n' "$revision" "$SOURCE_DATE_EPOCH"
