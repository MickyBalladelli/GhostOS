# SynOS Virtual Machine Improvement TODO

This is the next VM roadmap after `virtual_machine/TODO.md`. The old VM
roadmap marks its implementation and test plan complete. This file covers the
remaining proof, portability, safety, and fidelity work.

## Review baseline

- [x] Inspect all VM source, documentation, examples, and test files.
- [x] Compare the VM TODO claims with the public API, CLI, storage, firmware,
      terminal, snapshot, acceleration, and test-inventory code.
- [x] Keep the source-module inventory synchronized with the actual tree. The
      old Project Structure section names files that are now consolidated or
      no longer exist.

## P0: CPU and device correctness

- [x] Make malformed or unsupported guest instruction encodings return a
      structured decode or CPU exception instead of panicking.
- [x] Add differential instruction tests for supported x86_64 instructions,
      flags, privilege transitions, segmentation, paging faults, interrupts,
      string operations, and reset state.
- [x] Define the exact unsupported-instruction policy: unsupported decoded
      instructions return a VM error, without injecting an exception or
      halting the guest. Test it consistently in BIOS and long mode.
- [x] Centralize DMA and MMIO validation so every device rejects overflow,
      wraparound, unaligned ranges where forbidden, bad ownership, and stale
      descriptor chains with the same error contract.
- [x] Add device-model tests for ordering, interrupt coalescing, queue
      backpressure, reset during I/O, hot removal, and completion after reset.
- [x] Compare AHCI, NVMe, and Virtio block behavior for identical reads,
      writes, flushes, failures, and guest-visible capacity.

## P0: Snapshot and migration safety

- [x] Inventory every mutable VM field and mark it as serialized, rebuilt, or
      intentionally excluded. Include device queues, serial/input buffers,
      timers, RNG state, disks, network topology, guest-agent state, hotplug,
      acceleration mode, and firmware state.
- [x] Add snapshot schema negotiation, bounded decoding, explicit feature flags,
      migration compatibility, and a documented upgrade path.
- [x] Replace the current checksum-only trust model with authenticated
  integrity for snapshots and migration streams.
- [x] Authenticate migration peers, add connection and read/write timeouts,
      limit memory before allocation, and reject replayed or stale checkpoints.
- [x] Publish received checkpoints with file and directory sync ordering, then
      verify them after reopen. Preserve the old target on every failure.
- [x] Add migration tests for interrupted transfer, malicious length, corrupt
      pages, wrong VM topology, incompatible device state, duplicate delivery,
      and destination crash.

## P0: Disk and lock reliability

- [ ] Make stale-lock recovery the only public recovery path, or require an
      explicit force token for unconditional lock removal.
- [ ] Store canonical image identity, owner identity, start time, host identity,
      and format in the lock record. Handle PID reuse and cross-host copies.
- [ ] Add lock tests for concurrent open, crash, PID reuse, copied images,
      read-only attachment, symlink/path aliasing, and permission failure.
- [ ] Make disk creation, format writes, system-disk manifests, migration
      checkpoints, and lock publication crash-safe with file and directory
      sync ordering.
- [ ] Verify RAW, fixed VHD, and QCOW2 behavior against independent fixtures,
      including sparse files, malformed metadata, maximum sizes, discard,
      flush, and read-only/COW isolation.
- [ ] Add a repair and inspection report that never mutates the image unless a
      separate repair command is explicitly selected.

## P1: Terminal and host portability

- [ ] Replace the `stty` subprocess path with a platform terminal abstraction
      that restores settings on success, error, panic, signal, EOF, and child
      failure.
- [ ] Define Windows, macOS, Linux, and non-TTY behavior for raw input,
      resize, escape sequences, output flushing, and Ctrl-C.
- [ ] Add terminal tests with fake input/output and a child-process cleanup
      harness. Prove no host terminal state remains changed after every exit.
- [ ] Separate terminal policy from serial and PS/2 emulation, and expose a
      deterministic input transcript for replay.
- [ ] Add structured terminal session diagnostics without writing guest escape
      bytes into host error messages.

## P1: Execution, time, and acceleration parity

- [ ] Add an injectable monotonic clock for PIT, HPET, APIC, PV clock, timers,
      terminal polling, and guest scheduling.
- [ ] Add deterministic replay of instruction inputs, interrupts, device
      completions, timers, and host-facing input.
- [ ] Prove translation-cache invalidation for self-modifying code, page-table
      changes, permission changes, code aliases, interrupts, reset, and
      snapshot restore.
- [ ] Define JIT/translated execution equivalence with the interpreter and add
      differential runs for CPU state, memory, exceptions, and device effects.
- [ ] Make hardware acceleration report supported features, limitations, and
      fallback behavior. Never silently change correctness semantics.
- [ ] Add bounded benchmarks for decode, translation, memory, interrupt load,
      storage, network, and terminal workloads with repeatable machine data.

## P1: CLI, monitor, and remote control security

- [ ] Add typed command parsing and structured output for VM status, devices,
      disks, snapshots, migration, and monitor responses.
- [ ] Authenticate and authorize monitor clients. Restrict save, quit, device,
      disk, and migration actions separately.
- [ ] Bound monitor commands, connection lifetime, request size, response size,
      and concurrent clients. Test malformed and partial commands.
- [ ] Add migration encryption or a documented secure transport requirement,
      peer authorization, replay protection, and audit events.
- [ ] Redact host paths, credentials, image metadata, and guest data from
      diagnostics unless explicitly requested by an authorized operator.

## P1: Test evidence that matches reality

- [ ] Generate VM inventory entries from source modules and public APIs, then
      fail CI when a new device or API has no named test.
- [ ] Replace static test names with executed evidence containing command,
      revision, host, firmware, CPU count, image digest, and result state.
- [ ] Add decoder, device, disk-image, snapshot, terminal, and migration fuzz
      targets with retained crashing inputs and deterministic replay.
- [ ] Add QEMU evidence for BIOS, UEFI, one CPU, SMP, attached disks, reboot,
      shutdown, terminal wakeup, snapshot restore, and failure cleanup.
- [ ] Add cross-platform CI for Linux, macOS, and Windows host behavior, with
      explicit skips for unavailable acceleration and QEMU prerequisites.
- [ ] Add repeated soak runs that detect file, socket, process, lock, memory,
      translation-cache, and terminal-state leaks.

## P2: Documentation and release quality

- [x] Regenerate the Project Structure section from the current source tree.
- [ ] Document the public VM API invariants, device map, interrupt routes,
      snapshot schema, disk formats, migration protocol, and terminal contract.
- [ ] Add compatibility matrices for VM snapshots, system disks, guest boot
      images, device models, and CLI options.
- [ ] Make release artifacts include image digests, firmware mode, device
      topology, test evidence, and known host limitations.
- [ ] Add a changelog rule for guest-visible behavior changes and snapshot or
      disk-format changes.

## Immediate acceptance gate

- [ ] No malformed guest bytes or external command input can panic the VM.
- [ ] A snapshot restores the same guest-visible state, or clearly reports why
      excluded host state must be rebuilt.
- [ ] A migration stream is bounded, authenticated, replay-safe, and atomic at
      the destination.
- [ ] A crashed VM cannot leave a live disk lock or modified host terminal.
- [ ] Fast, QEMU, fuzz, and soak evidence is separated and each result says
      `passed`, `failed`, or `skipped` with a reason.
