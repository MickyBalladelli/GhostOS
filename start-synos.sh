#!/usr/bin/env bash
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$SCRIPT_DIR"

DISK_PATH=./virtual_machine/state/data.raw
DEFAULT_DISK_PATH=$DISK_PATH
PERSISTENCE_ARGS=()
VM_NAME=default
FORCE_NEW=false

if [ "${1:-}" = --new ]; then
  FORCE_NEW=true
  shift
fi

if [ "${1:-}" = --vm ]; then
  if [ "$#" -lt 2 ]; then
    echo "start-synos.sh: --vm needs a name" >&2
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
    echo "start-synos.sh: VM name must use letters, numbers, _ or -" >&2
    exit 2
    ;;
  *)
    DISK_PATH=./virtual_machine/state/$VM_NAME.raw
    if [ ! -f "$DISK_PATH" ]; then
      if [ ! -f "$DEFAULT_DISK_PATH" ]; then
        echo "start-synos.sh: default disk is missing: $DEFAULT_DISK_PATH" >&2
        exit 1
      fi
      cp "$DEFAULT_DISK_PATH" "$DISK_PATH"
    fi
    ;;
esac

if [ "$FORCE_NEW" = true ]; then
  PERSISTENCE_ARGS=(--copy-on-write)
else
  LOCK_STATUS=$(./target/release/synos-vm disk lock "$DISK_PATH" 2>/dev/null || true)
  case "$LOCK_STATUS" in
    "disk lock: active"*)
      if [ "$VM_NAME" = default ]; then
        PERSISTENCE_ARGS=(--copy-on-write)
      else
        echo "start-synos.sh: VM $VM_NAME is already running" >&2
        exit 1
      fi
      ;;
    "disk lock: stale"*)
      ./target/release/synos-vm disk recover-lock "$DISK_PATH"
      ;;
  esac
fi

KERNEL_PATH=./build/bios/kernel.bin
if [ ! -f "$KERNEL_PATH" ] || find ./kernel ./crates ./boot/bios -type f -newer "$KERNEL_PATH" -print -quit | grep -q .; then
  echo "Building stale BIOS image..." >&2
  ./scripts/build-bios-image.sh >/dev/null
fi

VM_COMMAND=(
  ./target/release/synos-vm \
  --kernel "$KERNEL_PATH" \
  --disk "$DISK_PATH" \
  --disk-size 64M \
  --disk-format raw \
  --disk-controller ahci
)

if [ "${#PERSISTENCE_ARGS[@]}" -gt 0 ]; then
  VM_COMMAND+=("${PERSISTENCE_ARGS[@]}")
fi

VM_COMMAND+=(--firmware bios --interactive --input ps2 "$@")
exec "${VM_COMMAND[@]}"
