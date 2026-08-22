#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$project_root/scripts/reproducible-env.sh"

cd "$project_root"
cargo build --locked --release -p ghostos-uefi --target x86_64-unknown-uefi
