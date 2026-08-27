#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$project_root/scripts/reproducible-env.sh"

"$project_root/scripts/build-bios-image.sh"

(
    cd "$project_root"
    cargo build -p ghostos-vm --release
)

(
    cd "$project_root"
    cargo test
)
