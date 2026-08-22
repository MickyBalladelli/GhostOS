#!/usr/bin/env bash
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/../.." && pwd)
KERNEL=${GHOSTOS_KERNEL:-$REPO_ROOT/kernel/build/bios/kernel.bin}
INITRD=${GHOSTOS_INITRD:-$REPO_ROOT/kernel/build/bios/initrd.img}
STEPS=${GHOSTOS_VM_STEPS:-1000000}

exec cargo run --release --manifest-path "$REPO_ROOT/virtual_machine/Cargo.toml" -- \
  --firmware bios \
  --kernel "$KERNEL" \
  --initrd "$INITRD" \
  --memory 128M \
  --append "console=serial0 net=virtio" \
  --steps "$STEPS"
