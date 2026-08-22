#!/usr/bin/env bash
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$SCRIPT_DIR"

SYSTEM_DISK_PATH=./virtual_machine/state/system.raw
DATA_DISK_PATH=./virtual_machine/state/data.raw
DEFAULT_DATA_DISK_PATH=$DATA_DISK_PATH
PERSISTENCE_ARGS=()
VM_NAME=default
FORCE_NEW=false

if [ "${1:-}" = --new ]; then
  FORCE_NEW=true
  shift
fi

if [ "${1:-}" = --vm ]; then
  if [ "$#" -lt 2 ]; then
    echo "start-ghostos.sh: --vm needs a name" >&2
    exit 2
  fi
  VM_NAME=$2
  shift 2
elif [ "${1:-}" != "" ] && [ "${1#-}" = "$1" ]; then
  VM_NAME=$1
  shift
fi

case "$VM_NAME" in
  default)
    ;;
  ""|.*|*[!A-Za-z0-9_-]*)
    echo "start-ghostos.sh: VM name must use letters, numbers, _ or -" >&2
    exit 2
    ;;
  *)
    SYSTEM_DISK_PATH=./virtual_machine/state/$VM_NAME-system.raw
    DATA_DISK_PATH=./virtual_machine/state/$VM_NAME.raw
    if [ ! -f "$DATA_DISK_PATH" ]; then
      if [ ! -f "$DEFAULT_DATA_DISK_PATH" ]; then
        echo "start-ghostos.sh: default data disk is missing: $DEFAULT_DATA_DISK_PATH" >&2
        exit 1
      fi
      cp "$DEFAULT_DATA_DISK_PATH" "$DATA_DISK_PATH"
    fi
    ;;
esac

KERNEL_PATH=./build/bios/kernel.bin
if [ ! -f "$KERNEL_PATH" ] || find ./kernel ./crates ./boot/bios ./userspace/boot-services -type f -newer "$KERNEL_PATH" -print -quit | grep -q .; then
  echo "Building stale BIOS image..." >&2
  ./scripts/build-bios-image.sh >/dev/null
fi

if [ ! -f "$SYSTEM_DISK_PATH" ]; then
  echo "Provisioning GhostOS system disk: $SYSTEM_DISK_PATH" >&2
  ./target/release/ghostos-vm disk provision "$SYSTEM_DISK_PATH" \
    --kernel "$KERNEL_PATH" \
    --size 64M \
    --boot-args console=serial0 \
    --machine-id "$VM_NAME" \
    --network-id "$VM_NAME"
elif ! ./target/release/ghostos-vm disk validate "$SYSTEM_DISK_PATH" >/dev/null 2>&1; then
  echo "start-ghostos.sh: system disk is invalid: $SYSTEM_DISK_PATH" >&2
  echo "start-ghostos.sh: move it aside, then start again to provision a new one" >&2
  exit 1
fi

if [ "$FORCE_NEW" = true ]; then
  PERSISTENCE_ARGS=(--copy-on-write)
else
  for LOCKED_DISK in "$SYSTEM_DISK_PATH" "$DATA_DISK_PATH"; do
    LOCK_STATUS=$(./target/release/ghostos-vm disk lock "$LOCKED_DISK" 2>/dev/null || true)
    case "$LOCK_STATUS" in
      "disk lock: active"*)
        if [ "$VM_NAME" = default ]; then
          PERSISTENCE_ARGS=(--copy-on-write)
        else
          echo "start-ghostos.sh: VM $VM_NAME is already running" >&2
          exit 1
        fi
        ;;
      "disk lock: stale"*)
        ./target/release/ghostos-vm disk recover-lock "$LOCKED_DISK"
        ;;
    esac
  done
fi

VM_COMMAND=(
  ./target/release/ghostos-vm \
  --system-disk "$SYSTEM_DISK_PATH" \
  --disk "$DATA_DISK_PATH" \
  --disk-size 64M \
  --disk-format raw \
  --disk-controller virtio-blk
)

if [ "${#PERSISTENCE_ARGS[@]}" -gt 0 ]; then
  VM_COMMAND+=("${PERSISTENCE_ARGS[@]}")
fi

VM_COMMAND+=(--firmware bios --interactive --input ps2 "$@")
exec "${VM_COMMAND[@]}"
