#!/usr/bin/env bash
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/../.." && pwd)
KERNEL=${GHOSTOS_KERNEL:-$REPO_ROOT/kernel/build/bios/kernel.bin}
INITRD=${GHOSTOS_INITRD:-$REPO_ROOT/kernel/build/bios/initrd.img}

exec cargo run --release --manifest-path "$REPO_ROOT/virtual_machine/Cargo.toml" -- \
  --firmware bios \
  --kernel "$KERNEL" \
  --initrd "$INITRD" \
  --memory 256M \
  --cpus 2 \
  --append "console=serial0"
