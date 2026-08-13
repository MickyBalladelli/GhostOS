# 4. Boot, Memory, and the Microkernel

The boot path has one purpose: turn firmware facts into a stable kernel entry contract.

## The boot contract

Both BIOS and UEFI paths pass a versioned `BootInfo` pointer to the same kernel entry. The record carries the facts the kernel needs:

- boot protocol version and magic;
- memory map;
- framebuffer information;
- command line and optional initrd information;
- firmware and platform data.

The kernel must validate the record before using it. Unknown versions, bad alignment, overlapping regions, impossible lengths, and unknown enum values are errors, not invitations to guess.

## BIOS path

The BIOS flow is split into small steps:

1. BIOS enters the stage-1 boot sector.
2. Stage 1 uses INT 13h extensions to load stage 2.
3. Stage 2 enables A20 and gathers the E820 memory map.
4. Stage 2 selects a VESA mode when available.
5. Stage 2 builds the boot information record.
6. Stage 2 enters long mode and jumps to `_start`.

Build a BIOS image from the repository root:

```sh
./scripts/build-bios-image.sh
```

The image is written to `build/bios/synos-bios.img`. The script assembles the 16-bit stages, builds the `x86_64-unknown-none` kernel, extracts its binary, checks the staging-sector limit, and writes the image with provenance.

## UEFI path

The UEFI loader validates and loads a PE/COFF application, captures the firmware memory map, passes the GOP framebuffer, exits boot services, and calls the same kernel entry. The loader also supports initrd and command-line handoff and has a chainload path for compatible firmware workflows.

```sh
./scripts/build-uefi-loader.sh
```

The EFI program lands under the target directory for `x86_64-unknown-uefi`.

The important design choice is shared handoff. BIOS and UEFI differ in how they obtain facts, but the kernel should not need two operating systems after entry.

## Early kernel startup

The kernel startup sequence is intentionally plain:

1. initialize serial and VGA/framebuffer console;
2. create the early frame allocator;
3. install an identity-mapped hardware root page table;
4. install interrupt handlers;
5. enable interrupts;
6. create the initial address spaces and services;
7. enter scheduling and runtime control.

The Ring 0 crate contains no general driver framework, filesystem, network stack, POSIX layer, or dynamic allocator. Those belong to user space.

## Memory model

`EarlyFrameAllocator` walks firmware-provided usable `MemoryRegion` entries and returns aligned 4 KiB frames. It skips reserved regions and starts above the early allocation floor. Quota-aware variants charge a tenant before allocating and refund the charge if physical allocation fails.

The architecture backends cover:

- x86_64 page tables, interrupts, port I/O, serial, framebuffer, and BIOS-era hardware;
- AArch64 `TTBR0_EL1` setup;
- RISC-V Sv39 root setup through `satp`;
- unsupported-target stubs that preserve a clear compile boundary.

## Page faults and address spaces

An address space owns mappings and the capabilities needed to create them. Page faults are not just crashes. A fault can represent:

- a missing page;
- an access-right violation;
- a remote DSM fetch;
- a copy-on-write transition;
- an invalid user pointer;
- a fatal protection fault.

The resolver must preserve the caller’s authority. A missing page does not mean “map anything.”

## Console and power

The kernel console writes serial output on x86 and uses a framebuffer or VGA fallback. It parses bounded ANSI sequences for terminal behavior and keeps console writes serialized.

Power handling has explicit shutdown, reboot, watchdog, and reset paths. ACPI services validate firmware tables and expose safe power operations. Legacy reset remains a fallback rather than the primary policy.

## Easy example: the machine handoff

Think of `BootInfo` as a hotel check-in card:

```text
firmware says: here is RAM, here is the screen, here is your luggage
kernel checks: the card is real, aligned, complete, and not contradictory
kernel starts: now firmware is no longer the system’s owner
```

If the card is corrupt, the kernel stops at the boundary. It does not build a system on a lie.

