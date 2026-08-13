#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$project_root/scripts/reproducible-env.sh"

"$project_root/scripts/build-bios-image.sh"

(
    cd "$project_root/virtual_machine"
    cargo build --locked --release
)

(
    cd "$project_root"
    cargo test
)
