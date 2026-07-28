#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
build_dir="$project_root/build/bios"
target_dir="$project_root/target/x86_64-unknown-none/release"
rust_tools_dir=$(dirname "$(rustc --print target-libdir)")/bin
rust_lld="$rust_tools_dir/rust-lld"
llvm_objcopy="$rust_tools_dir/llvm-objcopy"

mkdir -p "$build_dir"

clang --target=i386-unknown-none-elf -m16 -c \
    "$project_root/boot/bios/stage1.S" -o "$build_dir/stage1.o"
"$rust_lld" -flavor gnu -m elf_i386 --image-base=0 \
    --oformat binary -Ttext 0x7c00 \
    "$build_dir/stage1.o" -o "$build_dir/stage1.bin"

clang --target=i386-unknown-none-elf -m16 -c \
    "$project_root/boot/bios/stage2.S" -o "$build_dir/stage2.o"
"$rust_lld" -flavor gnu -m elf_i386 --image-base=0 \
    -T "$project_root/boot/bios/linker.ld" \
    "$build_dir/stage2.o" -o "$build_dir/stage2.bin"

cargo build --release -p synos-kernel --bin synos-kernel \
    --target x86_64-unknown-none

"$llvm_objcopy" -O binary "$target_dir/synos-kernel" "$build_dir/kernel.bin"

stage2_size=$(wc -c < "$build_dir/stage2.bin")
kernel_size=$(wc -c < "$build_dir/kernel.bin")

if [ "$stage2_size" -gt 8192 ]; then
    echo "stage2 exceeds its 16-sector reservation" >&2
    exit 1
fi

if [ "$kernel_size" -gt 262144 ]; then
    echo "kernel exceeds its 512-sector BIOS reservation" >&2
    exit 1
fi

dd if=/dev/zero of="$build_dir/synos-bios.img" bs=512 count=529 status=none
dd if="$build_dir/stage1.bin" of="$build_dir/synos-bios.img" conv=notrunc status=none
dd if="$build_dir/stage2.bin" of="$build_dir/synos-bios.img" bs=512 seek=1 conv=notrunc status=none
dd if="$build_dir/kernel.bin" of="$build_dir/synos-bios.img" bs=512 seek=17 conv=notrunc status=none

echo "$build_dir/synos-bios.img"
