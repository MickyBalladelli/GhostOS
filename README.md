# SynOS

SynOS now has an initial `no_std` bare-metal bootstrap for x86_64 BIOS and
UEFI machines, plus early page-table backends for x86_64, AArch64, and
RISC-V 64.

## Boot contract

Both x86 boot paths pass a versioned `BootInfo` pointer to the kernel:

- BIOS stage 1 loads stage 2 through INT 13h extensions.
- BIOS stage 2 enables A20, reads the E820 map, enters long mode, and jumps to
  the kernel `_start`.
- The UEFI loader captures the firmware memory map, exits boot services, and
  calls the same kernel entry.

The kernel starts serial/VGA output, creates an early frame allocator, installs
an identity-mapped hardware root page table, loads interrupt handlers, and then
enables interrupts.

## Build

Install the Rust targets and LLVM tools listed in `rust-toolchain.toml`, plus
`clang`. The image script uses Rust's bundled linker and object copier.

Build a BIOS disk image:

```sh
./scripts/build-bios-image.sh
```

The image is written to `build/bios/synos-bios.img`.

Build the UEFI application:

```sh
cargo uefi --release
```

The EFI executable is written under
`target/x86_64-unknown-uefi/release/synos-loader.efi`.

The AArch64 backend programs `TTBR0_EL1`; the RISC-V backend programs an Sv39
root through `satp`. Platform-specific firmware entry shims for those machines
can hand their memory map to the same `kernel_entry`.

## Microkernel core

The Ring 0 crate contains only boot, memory, interrupt, IPC, and scheduling
mechanisms. Drivers, filesystems, networking, policy, and identity belong in
isolated user address spaces.

- `task` defines fixed-size thread contexts and distinct kernel/user execution
  modes without a kernel heap.
- `scheduler` provides 64 generation-checked thread slots. Cooperative threads
  run until they yield or block. Real-time threads use fixed priority, then
  earliest deadline, and can preempt lower-ranked work on a timer tick.
- `ipc` is a bounded, non-blocking MPMC queue. Messages contain small control
  words plus shared-region descriptors, so payload bytes stay in mapped pages
  instead of being copied through the kernel.
- `capability` keeps 256 generation-checked object tokens and their derivation
  links in fixed kernel memory. Address spaces must present fine-grained rights
  for thread creation, memory mapping, and IPC. Delegation can only attenuate
  rights, and a parent token can revoke its complete delegation subtree.

This core has no driver, filesystem, network stack, dynamic allocator, or POSIX
compatibility layer. Those are user-space services communicating over IPC.

## SynFS day-one core

`synos-synfs` is a `no_std`, fixed-capacity filesystem core for the Ring 3
SynFS service. Metadata lives in immutable Copy-on-Write B+tree blocks. A write
creates the next file version, so `notes.txt`, `notes.txt;0`, and the highest
numbered version resolve to the latest contents while `notes.txt;1` selects an
exact immutable version.

File data is split into content-matched blocks. Unchanged tails are shared
between versions, and a failed or superseded tree update cannot damage the
committed root. `SynfsPurged` applies bounded per-file retention work, then
mark-and-sweep collection reclaims tombstoned data and abandoned CoW branches.

## Boot from USB

This bootstrap is experimental. Use a spare USB drive. The commands below erase
the selected drive completely. Check the drive name and size twice before
running them.

There are two different boot methods. The current build does not make one
hybrid USB that supports both.

### Legacy BIOS

Use this method on an x86_64 computer with Legacy Boot or CSM enabled. Secure
Boot must be disabled.

Build the image:

```sh
./scripts/build-bios-image.sh
```

Do not copy individual files to the USB drive. Write the complete
`build/bios/synos-bios.img` image to the whole drive, not to a partition.

On macOS:

```sh
diskutil list
diskutil unmountDisk /dev/diskN
sudo dd if=build/bios/synos-bios.img of=/dev/rdiskN bs=4m
sync
diskutil eject /dev/diskN
```

Replace `diskN` with the USB drive.

On Linux:

```sh
lsblk -p
sudo umount /dev/sdX1
sudo dd if=build/bios/synos-bios.img of=/dev/sdX bs=4M status=progress conv=fsync
sudo eject /dev/sdX
```

Replace `sdX` with the whole USB drive. Unmount every mounted partition if the
drive has more than one.

Boot the computer's one-time boot menu and choose the USB drive under its
Legacy or CSM entry. A successful start prints `SynOS kernel bootstrap` on VGA
and COM1 serial.

### UEFI

Use this method on an x86_64 UEFI computer. Secure Boot must be disabled because
the loader is not signed.

Build the UEFI application:

```sh
cargo uefi --release
```

Format the USB drive as GPT with a FAT32 partition. On macOS:

```sh
diskutil list
diskutil eraseDisk FAT32 SYNOS GPT /dev/disk5
mkdir -p /Volumes/SYNOS/EFI/BOOT
cp target/x86_64-unknown-uefi/release/synos-loader.efi \
    /Volumes/SYNOS/EFI/BOOT/BOOTX64.EFI
sync
diskutil eject /dev/disk5
```

On Linux, after creating and mounting a FAT32 EFI System Partition:

```sh
mkdir -p /Volumes/SYNOS/EFI/BOOT
cp target/x86_64-unknown-uefi/release/synos-loader.efi \
  /Volumes/SYNOS/EFI/BOOT/BOOTX64.EFI

sync
diskutil eject /dev/disk5
```

Only one file is copied. It must have this exact path and name:

```text
EFI/BOOT/BOOTX64.EFI
```

Choose the UEFI USB entry in the firmware boot menu. Current UEFI screen output
shows a `SynOS bare-metal bootstrap` prompt. Press a key to start the kernel.
If firmware handoff fails, the loader displays the EFI status instead of
silently returning to another operating system.

After rebuilding, always replace `EFI/BOOT/BOOTX64.EFI` on the USB drive with
the new `synos-loader.efi`.

### Serial console

COM1 uses 38400 baud, 8 data bits, no parity, and 1 stop bit. A USB-to-serial
adapter connected to the target machine can capture the earliest boot output.
