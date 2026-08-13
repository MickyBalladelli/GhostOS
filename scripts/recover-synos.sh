#!/bin/sh
set -eu

usage() {
    echo "usage: $0 --mode bios|uefi --target DEVICE_OR_IMAGE [--yes] [--dry-run]" >&2
    exit 2
}

mode=
target=
confirmed=0
dry_run=0
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

find_image() {
    name=$1
    if [ -f "$script_dir/$name" ]; then
        printf '%s\n' "$script_dir/$name"
    elif [ -f "$script_dir/../payload/$name" ]; then
        printf '%s\n' "$script_dir/../payload/$name"
    else
        echo "missing recovery image: $name" >&2
        exit 1
    fi
}

verify_image() {
    image_path=$1
    checksum_file=
    for candidate in "$script_dir/SHA256SUMS" "$script_dir/MEDIA-SHA256SUMS" \
        "$script_dir/../SHA256SUMS" "$script_dir/../MEDIA-SHA256SUMS"; do
        if [ -f "$candidate" ]; then
            checksum_file=$candidate
            break
        fi
    done
    [ -n "$checksum_file" ] || return 0
    expected=$(awk -v name="$(basename "$image_path")" '$2 == name { print $1; exit }' "$checksum_file")
    [ -n "$expected" ] || return 0
    if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "$image_path" | awk '{ print $1 }')
    else
        actual=$(shasum -a 256 "$image_path" | awk '{ print $1 }')
    fi
    [ "$actual" = "$expected" ] || {
        echo "checksum mismatch: $image_path" >&2
        exit 1
    }
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --mode)
            [ "$#" -ge 2 ] || usage
            mode=$2
            shift 2
            ;;
        --target)
            [ "$#" -ge 2 ] || usage
            target=$2
            shift 2
            ;;
        --yes)
            confirmed=1
            shift
            ;;
        --dry-run)
            dry_run=1
            shift
            ;;
        *)
            usage
            ;;
    esac
done

[ "$mode" = bios ] || [ "$mode" = uefi ] || usage
[ -n "$target" ] || usage

case "$mode" in
    bios) image=$(find_image synos-bios.img) ;;
    uefi) image=$(find_image synos-uefi.img) ;;
esac

verify_image "$image"

if [ "$dry_run" -eq 1 ]; then
    echo "would restore $image to $target"
    exit 0
fi

if [ "$confirmed" -ne 1 ]; then
    echo "refusing to restore $target without --yes" >&2
    exit 1
fi

if [ ! -e "$target" ]; then
    echo "target does not exist: $target" >&2
    exit 1
fi

echo "Restoring bootable SynOS $mode image to $target"
if [ ! -b "$target" ]; then
    truncate -s 0 "$target"
fi
dd if="$image" of="$target" bs=4M conv=fsync
echo "recovery restore complete; eject the target before booting it"
