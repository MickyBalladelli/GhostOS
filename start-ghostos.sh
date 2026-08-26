#!/usr/bin/env bash
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$SCRIPT_DIR"

SYSTEM_DISK_PATH=./virtual_machine/state/system.raw
DATA_DISK_PATH=./virtual_machine/state/data.raw
DATA_DISK_ARGS=()
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
    ;;
esac

if [ ! -f "$DATA_DISK_PATH" ]; then
  DATA_DISK_ARGS=(--create-if-missing)
fi

KERNEL_PATH=./build/bios/kernel.bin
if [ ! -f "$KERNEL_PATH" ] || find ./kernel ./crates ./boot/bios ./userspace/boot-services -type f -newer "$KERNEL_PATH" -print -quit | grep -q .; then
  echo "Building stale BIOS image..." >&2
  ./scripts/build-bios-image.sh >/dev/null
fi

SERVICE_IMAGE_DIR=./target/x86_64-unknown-none/release/build
SHELL_SERVICE_PATH=$(find "$SERVICE_IMAGE_DIR" -type f -name ghostos-shell.bin -print -quit)
LOGIN_SERVICE_PATH=$(find "$SERVICE_IMAGE_DIR" -type f -name ghostos-login.bin -print -quit)
if [ -z "$SHELL_SERVICE_PATH" ] || [ -z "$LOGIN_SERVICE_PATH" ]; then
  echo "start-ghostos.sh: service packages are missing; rebuild the BIOS image" >&2
  exit 1
fi
SERVICE_PACKAGE_ARGS=(
  --service "9=$SHELL_SERVICE_PATH"
  --service "14=$LOGIN_SERVICE_PATH"
)

if [ ! -x ./target/release/ghostos-vm ] || find ./virtual_machine ./Cargo.toml ./Cargo.lock -type f -newer ./target/release/ghostos-vm -print -quit | grep -q .; then
  echo "Building ghostos-vm (release)..." >&2
  cargo build -p ghostos-vm --release >/dev/null
fi

SYSTEM_DISK_CREATED=false
if [ ! -f "$SYSTEM_DISK_PATH" ]; then
  echo "Provisioning GhostOS system disk: $SYSTEM_DISK_PATH" >&2
  ./target/release/ghostos-vm disk provision "$SYSTEM_DISK_PATH" \
    --kernel "$KERNEL_PATH" \
    --size 64M \
    --boot-args console=serial0 \
    --machine-id "$VM_NAME" \
    --network-id "$VM_NAME"
  SYSTEM_DISK_CREATED=true
else
  echo "$SYSTEM_DISK_PATH"
fi

SYSTEM_DISK_REFRESH_ALLOWED=true
if [ "$FORCE_NEW" = true ]; then
  PERSISTENCE_ARGS=(--copy-on-write)
  SYSTEM_DISK_REFRESH_ALLOWED=false
else
  for LOCKED_DISK in "$SYSTEM_DISK_PATH" "$DATA_DISK_PATH"; do
    LOCK_STATUS=$(./target/release/ghostos-vm disk lock "$LOCKED_DISK" 2>/dev/null || true)
    case "$LOCK_STATUS" in
      "disk lock: active"*)
        if [ "$LOCKED_DISK" = "$SYSTEM_DISK_PATH" ]; then
          SYSTEM_DISK_REFRESH_ALLOWED=false
        fi
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

if [ "$SYSTEM_DISK_CREATED" = false ] && [ "$SYSTEM_DISK_REFRESH_ALLOWED" = true ]; then
  ./target/release/ghostos-vm disk refresh-services "$SYSTEM_DISK_PATH" \
    "${SERVICE_PACKAGE_ARGS[@]}"
fi

VM_COMMAND=(
  ./target/release/ghostos-vm \
  --system-disk "$SYSTEM_DISK_PATH" \
  --disk "$DATA_DISK_PATH" \
  --disk-size 64M \
  --disk-format raw \
  --disk-controller virtio-blk \
  --kernel "$KERNEL_PATH" \
  --append console=serial0
)

if [ "${#PERSISTENCE_ARGS[@]}" -gt 0 ]; then
  VM_COMMAND+=("${PERSISTENCE_ARGS[@]}")
fi

if [ "${#DATA_DISK_ARGS[@]}" -gt 0 ]; then
  VM_COMMAND+=("${DATA_DISK_ARGS[@]}")
fi

VM_COMMAND+=(--firmware bios --interactive --input ps2 "$@")
exec "${VM_COMMAND[@]}"
