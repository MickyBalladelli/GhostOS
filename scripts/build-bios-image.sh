#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$project_root/scripts/reproducible-env.sh"
# RUSTFLAGS overrides target-specific config, so retain the kernel's required
# non-PIC link model when the reproducible path-remap flags are active.
RUSTFLAGS="$RUSTFLAGS -Ccode-model=kernel -Cno-redzone=yes -Crelocation-model=static -Clink-arg=-no-pie"
export RUSTFLAGS
build_dir="$project_root/build/bios"
target_dir="$project_root/target/x86_64-unknown-none/release"
source_revision=$(git -C "$project_root" rev-parse HEAD 2>/dev/null || printf 'unknown')
rust_tools_dir=$(dirname "$(rustc --print target-libdir)")/bin
rust_lld="$rust_tools_dir/rust-lld"
llvm_objcopy="$rust_tools_dir/llvm-objcopy"
stage2_sectors=16
# Staging load starts at 0x10000; leave 1 KiB below typical EBDA (~0x9FC00).
max_kernel_sectors=1148

mkdir -p "$build_dir"

clang --target=i386-unknown-none-elf -m16 -c \
    "$project_root/boot/bios/stage1.S" -o "$build_dir/stage1.o"
"$rust_lld" -flavor gnu -m elf_i386 --image-base=0 \
    --oformat binary -Ttext 0x7c00 \
    "$build_dir/stage1.o" -o "$build_dir/stage1.bin"

cargo build --locked --release -p synos-kernel --bin synos-kernel \
    --target x86_64-unknown-none

"$llvm_objcopy" -O binary "$target_dir/synos-kernel" "$build_dir/kernel.bin"

kernel_size=$(wc -c < "$build_dir/kernel.bin")
kernel_sectors=$(( (kernel_size + 511) / 512 ))

if [ "$kernel_sectors" -gt "$max_kernel_sectors" ]; then
    echo "kernel exceeds its BIOS staging-memory limit of $max_kernel_sectors sectors" >&2
    exit 1
fi

clang --target=i386-unknown-none-elf -m16 \
    -DKERNEL_SECTORS="$kernel_sectors" -DSTAGE2_SECTORS="$stage2_sectors" -c \
    "$project_root/boot/bios/stage2.S" -o "$build_dir/stage2.o"
"$rust_lld" -flavor gnu -m elf_i386 --image-base=0 \
    -T "$project_root/boot/bios/linker.ld" \
    "$build_dir/stage2.o" -o "$build_dir/stage2.bin"

stage2_size=$(wc -c < "$build_dir/stage2.bin")

if [ "$stage2_size" -gt 8192 ]; then
    echo "stage2 exceeds its 16-sector reservation" >&2
    exit 1
fi

image_sectors=$(( 1 + stage2_sectors + kernel_sectors ))
dd if=/dev/zero of="$build_dir/synos-bios.img" bs=512 count="$image_sectors" status=none
dd if="$build_dir/stage1.bin" of="$build_dir/synos-bios.img" conv=notrunc status=none
dd if="$build_dir/stage2.bin" of="$build_dir/synos-bios.img" bs=512 seek=1 conv=notrunc status=none
dd if="$build_dir/kernel.bin" of="$build_dir/synos-bios.img" bs=512 seek=$((1 + stage2_sectors)) conv=notrunc status=none
printf '%s\n' "$source_revision" > "$build_dir/synos-bios.img.revision"

echo "$build_dir/synos-bios.img"
