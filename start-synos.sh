#!/usr/bin/env bash
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$SCRIPT_DIR"

exec ./target/release/synos-vm \
  --kernel ./build/bios/kernel.bin \
  --disk ./virtual_machine/state/data.raw \
  --disk-size 64M \
  --disk-format raw \
  --disk-controller virtio-blk \
  --firmware bios --interactive
