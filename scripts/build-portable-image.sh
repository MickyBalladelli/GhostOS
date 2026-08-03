#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
loader="$project_root/target/x86_64-unknown-uefi/release/synos-loader.efi"
output_dir="$project_root/build/portable"
output="$output_dir/synos.img"
source_revision=$(git -C "$project_root" rev-parse HEAD 2>/dev/null || printf 'unknown')

if ! command -v mkfs.fat >/dev/null 2>&1; then
    echo "mkfs.fat is required (dosfstools)" >&2
    exit 1
fi

if ! command -v mmd >/dev/null 2>&1 || ! command -v mcopy >/dev/null 2>&1; then
    echo "mmd and mcopy are required (mtools)" >&2
    exit 1
fi

if [ ! -f "$loader" ]; then
    echo "Build the release UEFI loader first: cargo uefi --release" >&2
    exit 1
fi

mkdir -p "$output_dir"
temporary_image=$(mktemp "$output_dir/synos.img.XXXXXX")
truncate -s 64M "$temporary_image"
mkfs.fat -F 32 -n SYNOS "$temporary_image"
mmd -i "$temporary_image" ::/EFI
mmd -i "$temporary_image" ::/EFI/BOOT
mcopy -i "$temporary_image" "$loader" ::/EFI/BOOT/BOOTX64.EFI
mv "$temporary_image" "$output"
printf '%s\n' "$source_revision" > "$output.revision"

echo "$output"
