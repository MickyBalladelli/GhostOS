# SynOS TODO

Build the real operating system core before adding more advanced features.

## P0: Run user processes

- [x] Wire the application image loader to a real kernel process backend.
- [x] Create real Ring 3 address spaces.
- [x] Add user-mode entry and return paths.
- [x] Add syscall or call-gate entry and dispatch.
- [x] Add real register context switching.
- [x] Add process exit, wait, cancellation, and crash reporting.
- [x] Prove one native Ring 3 hello-world process runs after boot.

## P0: Start system services

- [x] Make the kernel start `synos-init` after early hardware setup.
- [x] Register and start the filesystem service.
- [x] Register and start the storage service.
- [x] Register and start the network service.
- [x] Register and start logging and audit services.
- [x] Register and start authentication and package services.
- [x] Connect service restart and fault fencing to real kernel processes.
- [x] Replace empty service entrypoints with working Ring 3 programs.
- [x] Boot to a service-owned shell instead of a kernel-owned shell.

## P0: Finish memory isolation

- [x] Replace the single identity-mapped address space with per-process page tables.
- [x] Add user/read/write/execute page permissions.
- [ ] Add kernel/user virtual address layout.
- [ ] Add physical frame ownership and reclamation.
- [ ] Add page mapping and unmapping syscalls.
- [ ] Add TLB invalidation and cross-CPU TLB shootdown.
- [ ] Add copy-on-write memory.
- [ ] Add guard pages, stack growth, and invalid-access termination.
- [ ] Add capability-checked DMA and IOMMU protection.
- [ ] Prove one process cannot read or write another process.

## P0: Integrate physical storage and drivers

- [ ] Discover PCI devices during boot.
- [ ] Start the PCI, AHCI, NVMe, and Ethernet driver services.
- [ ] Connect driver DMA and MMIO access to kernel capabilities.
- [ ] Add real block-device request and completion paths.
- [ ] Mount SynFS from a physical disk.
- [ ] Load the system manifest and service packages from SynFS.
- [ ] Add persistent filesystem recovery after power loss.
- [ ] Add USB host and USB storage support.
- [ ] Add basic real keyboard and mouse device support.
- [ ] Document unsupported hardware clearly.

## P0: Make installation and boot persistent

- [ ] Put the kernel, initrd, system manifest, and service packages on a system disk.
- [ ] Make the BIOS loader read the installed system image.
- [ ] Make the UEFI loader read the installed system image.
- [ ] Add signed boot artifacts and measured boot metadata.
- [ ] Add install, upgrade, rollback, and recovery commands.
- [ ] Validate that a machine can reboot and return to the same usable system.

## P1: Add authentication and security startup

- [ ] Add a boot login/session service.
- [ ] Create the first administrator identity safely.
- [ ] Load users, groups, capabilities, and policy from persistent storage.
- [ ] Connect passkey, TPM, and SSH authentication to real sessions.
- [ ] Add session logout, revocation, and timeout handling.
- [ ] Ensure the kernel shell cannot bypass authorization.
- [ ] Keep secrets out of logs, snapshots, and crash capsules.

## P1: Finish CPU and architecture support

- [ ] Bring up application processors on x86_64.
- [ ] Add local APIC and inter-processor interrupt routing.
- [ ] Add per-CPU scheduler and interrupt state.
- [ ] Add real AArch64 exception, syscall, and user-mode paths.
- [ ] Add real RISC-V trap, syscall, and user-mode paths.
- [ ] Replace AArch64 and RISC-V isolation stubs.
- [ ] Add architecture-specific boot and hardware evidence.

## P1: Make networking usable

- [ ] Connect physical NIC drivers to `synos-netd`.
- [ ] Add interface discovery and naming.
- [ ] Add DHCP and static network configuration at boot.
- [ ] Persist network configuration in SynFS.
- [ ] Add firewall policy activation during service startup.
- [ ] Add DNS and basic time synchronization services.
- [ ] Prove network recovery after link loss and service restart.

## P1: Make updates and recovery real

- [ ] Build the complete power-loss, disk-full, device-reset, and network-failure matrix.
- [ ] Add rolling, canary, blue/green, and emergency update strategies.
- [ ] Add boot-time automatic rollback after failed health checks.
- [ ] Add a recovery shell that works when normal services fail.
- [ ] Add backup restore that recreates a bootable system.
- [ ] Test repeated reboot, suspend, resume, hotplug, and service restart cycles.

## P2: Usability and compatibility

- [ ] Move normal filesystem commands out of Ring 0.
- [ ] Add a stable shell session and terminal service.
- [ ] Expand POSIX compatibility beyond the current small syscall set.
- [ ] Add process resource limits visible to users and operators.
- [ ] Add package installation and removal from the running system.
- [ ] Add clear hardware, service, and recovery diagnostics.
- [ ] Add documented support levels for x86_64, AArch64, and RISC-V.

## Definition of usable OS

- [ ] Boot from disk on real x86_64 hardware.
- [ ] Start isolated user-space services.
- [ ] Log in as an administrator.
- [ ] Create, read, write, and delete persistent files.
- [ ] Run two isolated applications at the same time.
- [ ] Use a network interface.
- [ ] Restart a failed service without rebooting the machine.
- [ ] Reboot and recover the same system state.
- [ ] Recover safely after a failed update.

## Extra system work

### Time and randomness

- [ ] Add a real-time clock.
- [ ] Add a monotonic clock shared by kernel and services.
- [ ] Add timers, sleep, and wakeup primitives.
- [ ] Add a trusted entropy and random-number service.

### IPC safety

- [ ] Define endpoint cleanup and ownership rules.
- [ ] Define shared-buffer lifetime and revocation rules.
- [ ] Add IPC backpressure and deadlock handling.
- [ ] Apply IPC quotas and fairness between processes.
- [ ] Add IPC tracing and stuck-request diagnostics.

### Security hardening

- [ ] Add ASLR and KASLR where supported.
- [ ] Enforce W^X for every process and loaded image.
- [ ] Add SMEP and SMAP protection on x86_64.
- [ ] Add stack protection and control-flow hardening.
- [ ] Add isolated secure key storage.
- [ ] Fuzz syscall and privilege-boundary inputs.
- [ ] Test for privilege escalation and confused-deputy bugs.

### Filesystem behavior

- [ ] Add process file-descriptor tables and open-handle rules.
- [ ] Define atomic rename and `fsync` guarantees.
- [ ] Add file and record locking through the filesystem service.
- [ ] Add symbolic links and permission checks.
- [ ] Add filesystem quotas visible to users and operators.
- [ ] Add mapped-file support with capability checks.

### Power management

- [ ] Add CPU idle-state management.
- [ ] Add suspend and resume on supported hardware.
- [ ] Add thermal throttling and thermal event reporting.
- [ ] Add battery and power-source reporting.
- [ ] Add watchdog-based recovery for hung services and CPUs.

### Observability and debugging

- [ ] Persist system and service logs across reboot.
- [ ] Export bounded metrics and tracing data.
- [ ] Add crash-dump storage and a crash viewer.
- [ ] Add boot-failure diagnostics.
- [ ] Add a capability-safe remote debugger.
- [ ] Add service dependency and startup diagnostics.

### Build and release

- [ ] Make release builds reproducible.
- [ ] Sign release images and boot artifacts.
- [ ] Generate SBOM and dependency provenance for each release.
- [ ] Produce complete installer and recovery artifacts.
- [ ] Validate upgrade compatibility before release.

### Testing gates

- [ ] Add real hardware boot evidence.
- [ ] Add service-start and service-restart integration tests.
- [ ] Add process-isolation integration tests.
- [ ] Add power-loss and disk-corruption tests.
- [ ] Add long-running soak tests for leaks and stale capabilities.
- [ ] Keep QEMU, hardware, fuzz, and soak results separate.

### Compatibility

- [ ] Version and document the syscall ABI.
- [ ] Version and document the user-space package ABI.
- [ ] Add migration tools for persistent system state.
- [ ] Add a stable user-space SDK compatibility policy.
