#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$project_root/scripts/reproducible-env.sh"

output_dir=${SYNOS_RELEASE_DIR:-$project_root/build/release}
case "$output_dir" in
    /*) ;;
    *) output_dir="$project_root/$output_dir" ;;
esac
mkdir -p "$output_dir"

if ! command -v mkfs.fat >/dev/null 2>&1 || ! command -v mmd >/dev/null 2>&1 || ! command -v mcopy >/dev/null 2>&1; then
    echo "mkfs.fat, mmd, and mcopy are required (dosfstools and mtools)" >&2
    exit 1
fi

"$project_root/scripts/build-bios-image.sh" >/dev/null
"$project_root/scripts/build-uefi-loader.sh" >/dev/null
"$project_root/scripts/build-portable-image.sh" >/dev/null

revision=$(git -C "$project_root" rev-parse HEAD 2>/dev/null || printf 'unknown')
loader="$project_root/target/x86_64-unknown-uefi/release/synos-loader.efi"
bios_image="$project_root/build/bios/synos-bios.img"
uefi_image="$project_root/build/portable/synos.img"

cp "$loader" "$output_dir/synos-loader.efi"
cp "$bios_image" "$output_dir/synos-bios.img"
cp "$uefi_image" "$output_dir/synos-uefi.img"
cp "$project_root/scripts/install-synos.sh" "$output_dir/install-synos.sh"
cp "$project_root/scripts/recover-synos.sh" "$output_dir/recover-synos.sh"
chmod 755 "$output_dir/install-synos.sh" "$output_dir/recover-synos.sh"

create_media() {
    kind=$1
    label=$2
    output="$output_dir/synos-$kind.img"
    volume_id=$(printf '%s%s' "$revision" "$kind" | cksum | awk '{ printf "%08x", $1 }' | cut -c1-8)

    truncate -s 256M "$output"
    mkfs.fat --invariant -F 32 -i "$volume_id" -n "$label" "$output" >/dev/null
    mmd -i "$output" ::/EFI
    mmd -i "$output" ::/EFI/BOOT
    mmd -i "$output" ::/payload
    mmd -i "$output" ::/tools
    mmd -i "$output" ::/docs
    mcopy -i "$output" "$loader" ::/EFI/BOOT/BOOTX64.EFI
    mcopy -i "$output" "$bios_image" ::/payload/synos-bios.img
    mcopy -i "$output" "$uefi_image" ::/payload/synos-uefi.img
    mcopy -i "$output" "$output_dir/MEDIA-SHA256SUMS" ::/MEDIA-SHA256SUMS
    mcopy -i "$output" "$output_dir/install-synos.sh" ::/tools/install-synos.sh
    mcopy -i "$output" "$output_dir/recover-synos.sh" ::/tools/recover-synos.sh
    mcopy -i "$output" "$project_root/docs/$kind-media.md" ::/docs/README.md
}

if command -v sha256sum >/dev/null 2>&1; then
    hash_file() { sha256sum "$1" | awk '{ print $1 }'; }
else
    hash_file() { shasum -a 256 "$1" | awk '{ print $1 }'; }
fi

{
    printf '%s  %s\n' "$(hash_file "$output_dir/synos-bios.img")" synos-bios.img
    printf '%s  %s\n' "$(hash_file "$output_dir/synos-uefi.img")" synos-uefi.img
    printf '%s  %s\n' "$(hash_file "$output_dir/synos-loader.efi")" synos-loader.efi
    printf '%s  %s\n' "$(hash_file "$output_dir/install-synos.sh")" install-synos.sh
    printf '%s  %s\n' "$(hash_file "$output_dir/recover-synos.sh")" recover-synos.sh
} > "$output_dir/MEDIA-SHA256SUMS"

create_media installer INSTALR
create_media recovery RECOVERY

python3 "$project_root/scripts/package-install-recovery.py" \
    --root "$project_root" \
    --output "$output_dir" \
    --revision "$revision" \
    --source-date-epoch "$SOURCE_DATE_EPOCH"

python3 "$project_root/scripts/validate-install-recovery-artifacts.py" \
    --directory "$output_dir"

printf '%s\n' "$revision" > "$output_dir/revision.txt"
printf '%s\n' "$output_dir"
