#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
disk=${1:-}
build_dir="$project_root/build/bios"
rust_tools_dir=$(dirname "$(rustc --print target-libdir)")/bin
rust_lld="$rust_tools_dir/rust-lld"
llvm_objcopy="$rust_tools_dir/llvm-objcopy"

if [ -z "$disk" ] || [ ! -f "$disk" ]; then
    echo "usage: $0 SYSTEM_DISK" >&2
    exit 2
fi

mkdir -p "$build_dir"
clang --target=i386-unknown-none-elf -m16 -c \
    "$project_root/boot/bios/stage1.S" -o "$build_dir/installed-stage1.o"
"$rust_lld" -flavor gnu -m elf_i386 --image-base=0 \
    --oformat binary -Ttext 0x7c00 \
    "$build_dir/installed-stage1.o" -o "$build_dir/installed-stage1.bin"

clang --target=i386-unknown-none-elf -m16 \
    -DKERNEL_SECTORS=1 -DSTAGE2_SECTORS=16 -c \
    "$project_root/boot/bios/stage2.S" -o "$build_dir/installed-stage2.o"
"$rust_lld" -flavor gnu -m elf_i386 --image-base=0 \
    -T "$project_root/boot/bios/linker.ld" \
    "$build_dir/installed-stage2.o" -o "$build_dir/installed-stage2.bin"

stage2_size=$(wc -c < "$build_dir/installed-stage2.bin")
if [ "$stage2_size" -gt 8192 ]; then
    echo "installed BIOS stage2 exceeds its 16-sector reservation" >&2
    exit 1
fi

dd if="$disk" of="$build_dir/installed-header.bin" bs=1 skip=448 count=36 status=none
dd if="$build_dir/installed-stage1.bin" of="$disk" conv=notrunc status=none
dd if="$build_dir/installed-header.bin" of="$disk" bs=1 seek=448 conv=notrunc status=none
dd if="$build_dir/installed-stage2.bin" of="$disk" bs=512 seek=1 conv=notrunc status=none

echo "$disk"
