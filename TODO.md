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
- [x] Add kernel/user virtual address layout.
- [x] Add physical frame ownership and reclamation.
- [x] Add page mapping and unmapping syscalls.
- [x] Add TLB invalidation and cross-CPU TLB shootdown.
- [x] Add copy-on-write memory.
- [x] Add guard pages, stack growth, and invalid-access termination.
- [x] Add capability-checked DMA and IOMMU protection.
- [x] Prove one process cannot read or write another process.

## P0: Integrate physical storage and drivers

- [x] Discover PCI devices during boot.
- [x] Start the PCI, AHCI, NVMe, and Ethernet driver services.
- [x] Connect driver DMA and MMIO access to kernel capabilities.
- [x] Add real block-device request and completion paths.
- [x] Mount SynFS from a physical disk.
- [x] Load the system manifest and service packages from SynFS.
- [x] Add persistent filesystem recovery after power loss.
- [x] Add USB host and USB storage support.
- [x] Add basic real keyboard and mouse device support.
- [x] Document unsupported hardware clearly.

## P0: Make installation and boot persistent

- [x] Put the kernel, initrd, system manifest, and service packages on a system disk.
- [x] Make the BIOS loader read the installed system image.
- [x] Make the UEFI loader read the installed system image.
- [x] Add signed boot artifacts and measured boot metadata.
- [x] Add install, upgrade, rollback, and recovery commands.
- [x] Validate that a machine can reboot and return to the same usable system.

## P1: Add authentication and security startup

- [x] Add a boot login/session service.
- [x] Create the first administrator identity safely.
- [x] Load users, groups, capabilities, and policy from persistent storage.
- [x] Connect passkey, TPM, and SSH authentication to real sessions.
- [x] Add session logout, revocation, and timeout handling.
- [x] Ensure the kernel shell cannot bypass authorization.
- [x] Keep secrets out of logs, snapshots, and crash capsules.

## P1: Finish CPU and architecture support

- [x] Bring up application processors on x86_64.
- [x] Add local APIC and inter-processor interrupt routing.
- [x] Add per-CPU scheduler and interrupt state.
- [x] Add real AArch64 exception, syscall, and user-mode paths.
- [x] Add real RISC-V trap, syscall, and user-mode paths.
- [x] Replace AArch64 and RISC-V isolation stubs.
- [x] Add architecture-specific boot and hardware evidence.

## P1: Make networking usable

- [x] Connect physical NIC drivers to `synos-netd`.
- [x] Add interface discovery and naming.
- [x] Add DHCP and static network configuration at boot.
- [x] Persist network configuration in SynFS.
- [x] Add firewall policy activation during service startup.
- [x] Add DNS and basic time synchronization services.
- [x] Prove network recovery after link loss and service restart.

## P1: Make updates and recovery real

- [x] Build the complete power-loss, disk-full, device-reset, and network-failure matrix.
- [x] Add rolling, canary, blue/green, and emergency update strategies.
- [x] Add boot-time automatic rollback after failed health checks.
- [x] Add a recovery shell that works when normal services fail.
- [x] Add backup restore that recreates a bootable system.
- [x] Test repeated reboot, suspend, resume, hotplug, and service restart cycles.

## P2: Usability and compatibility

- [x] Move normal filesystem commands out of Ring 0.
- [ ] Add a stable shell session and terminal service.
- [ ] Expand POSIX compatibility beyond the current small syscall set.
- [ ] Add process resource limits visible to users and operators.
- [ ] Add package installation and removal from the running system.
- [ ] Add clear hardware, service, and recovery diagnostics.
- [ ] Add documented support levels for x86_64, AArch64, and RISC-V.

## Definition of usable OS

- [x] Boot from disk on real x86_64 hardware.
- [x] Start isolated user-space services.
- [x] Log in as an administrator.
- [x] Create, read, write, and delete persistent files.
- [x] Run two isolated applications at the same time.
- [x] Use a network interface.
- [x] Restart a failed service without rebooting the machine.
- [x] Reboot and recover the same system state.
- [x] Recover safely after a failed update.

## Extra system work

### Time and randomness

- [x] Add a real-time clock.
- [x] Add a monotonic clock shared by kernel and services.
- [x] Add timers, sleep, and wakeup primitives.
- [x] Add a trusted entropy and random-number service.

### IPC safety

- [x] Define endpoint cleanup and ownership rules.
- [x] Define shared-buffer lifetime and revocation rules.
- [x] Add IPC backpressure and deadlock handling.
- [x] Apply IPC quotas and fairness between processes.
- [x] Add IPC tracing and stuck-request diagnostics.

### Security hardening

- [x] Add ASLR and KASLR where supported.
- [x] Enforce W^X for every process and loaded image.
- [x] Add SMEP and SMAP protection on x86_64.
- [x] Add stack protection and control-flow hardening.
- [x] Add isolated secure key storage.
- [x] Fuzz syscall and privilege-boundary inputs.
- [x] Test for privilege escalation and confused-deputy bugs.

### Filesystem behavior

- [x] Add process file-descriptor tables and open-handle rules.
- [x] Define atomic rename and `fsync` guarantees.
- [x] Add file and record locking through the filesystem service.
- [x] Add symbolic links and permission checks.
- [x] Add filesystem quotas visible to users and operators.
- [x] Add mapped-file support with capability checks.

### Power management

- [x] Add CPU idle-state management.
- [x] Add suspend and resume on supported hardware.
- [x] Add thermal throttling and thermal event reporting.
- [x] Add battery and power-source reporting.
- [x] Add watchdog-based recovery for hung services and CPUs.

### Observability and debugging

- [x] Persist system and service logs across reboot.
- [x] Export bounded metrics and tracing data.
- [x] Add crash-dump storage and a crash viewer.
- [x] Add boot-failure diagnostics.
- [x] Add a capability-safe remote debugger.
- [x] Add service dependency and startup diagnostics.

### Build and release

- [x] Make release builds reproducible.
- [x] Sign release images and boot artifacts.
- [x] Generate SBOM and dependency provenance for each release.
- [x] Produce complete installer and recovery artifacts.
- [x] Validate upgrade compatibility before release.

### Testing gates

- [x] Add real hardware boot evidence.
      Implementation: [`scripts/record-hardware-boot-evidence.py`](scripts/record-hardware-boot-evidence.py)
      packages a COM1 capture, bare-metal inventory, exact boot artifact hash,
      and Git revision. [`scripts/validate-hardware-boot-evidence.py`](scripts/validate-hardware-boot-evidence.py)
      rejects hypervisor declarations, panic output, incomplete Ring 3 boot,
      stale hashes, and mismatched BIOS/UEFI inventory. See
      [`platforms/README.md`](platforms/README.md).
- [x] Add service-start and service-restart integration tests.
- [x] Add process-isolation integration tests.
- [x] Add power-loss and disk-corruption tests.
- [x] Add long-running soak tests for leaks and stale capabilities.
- [x] Keep QEMU, hardware, fuzz, and soak results separate.

### Compatibility

- [x] Version and document the syscall ABI.
- [x] Version and document the user-space package ABI.
- [x] Add migration tools for persistent system state.
- [x] Add a stable user-space SDK compatibility policy.

## Login, first login, and account management

### Login surface

- [x] Add a user-space login service that owns the terminal login flow.
- [x] Add a `LOGIN` command or login screen to the service-owned shell.
- [x] Show a clear first-boot message when no administrator account exists.
- [x] Prompt for username and credential without echoing private input.
- [x] Support passkey login from the local terminal.
- [x] Support TPM-backed credential login from the local terminal.
- [x] Support SSH-key login for configured remote sessions.
- [x] Display useful failure messages without revealing whether an account exists.
- [x] Rate-limit failed login attempts.
- [x] Lock login temporarily after repeated failures.
- [x] Add `LOGOUT` and `WHOAMI` commands.
- [x] Return to the locked prompt after logout, timeout, or session revocation.
- [x] Connect successful auth sessions to shell authorization and capabilities.
- [x] Preserve session expiry, revocation, and identity changes across all shell paths.

### First login and administrator setup

- [x] Detect an unprovisioned system during boot.
- [x] Enter a restricted first-run setup mode before the normal shell starts.
- [x] Require physical-console access or an equivalent trusted bootstrap proof.
- [x] Create the first administrator username.
- [x] Register the first administrator passkey, TPM credential, or SSH key.
- [x] Require confirmation before committing the first administrator account.
- [x] Persist the first administrator atomically in SynFS.
- [x] Make first-admin creation safe to retry after power loss.
- [x] Prevent first-admin setup from replacing an existing account database.
- [x] Provide a recovery mode for an interrupted or failed first-login setup.
- [x] Provide a documented recovery procedure when the first administrator loses all credentials.
- [x] Audit first-admin creation, recovery, and cancellation events.

### Account lifecycle

- [x] Add `ACCOUNT LIST` with safe summaries of local accounts.
- [x] Add `ACCOUNT SHOW <username>` with permission-checked details.
- [x] Add `ACCOUNT CREATE <username>` for administrators.
- [x] Add `ACCOUNT DELETE <username>` with confirmation and last-admin protection.
- [x] Add `ACCOUNT ENABLE <username>` and `ACCOUNT DISABLE <username>`.
- [x] Add `ACCOUNT RENAME <old> <new>` with persistent identity rules.
- [x] Add account creation, update, disable, and deletion through the management API.
- [x] Enforce username syntax, length, normalization, and reserved-name rules.
- [x] Prevent deletion or disabling of the last usable administrator.
- [x] Revoke all sessions when an account is disabled or deleted.
- [x] Keep stable identity IDs when account display names change.
- [x] Add account state for active, disabled, locked, expired, and pending setup.

### Credentials and access policy

- [x] Add `CREDENTIAL LIST <username>` for authorized administrators.
- [x] Add `CREDENTIAL ADD <username>` for passkeys, TPM credentials, and SSH keys.
- [x] Add `CREDENTIAL REMOVE <username> <id>` with self-lockout protection.
- [x] Allow users to enroll and remove their own credentials under policy.
- [x] Store only credential public data and metadata, never private keys or secrets.
- [x] Add credential labels, creation time, last-used time, and revocation state.
- [x] Rotate and revoke credentials without deleting the account.
- [x] Support account expiration and credential expiration policies.
- [x] Add password login only if a password verifier and secure recovery policy exist.
- [x] Define administrator, operator, auditor, and read-only account roles.
- [x] Add group membership management with `GROUP LIST`, `GROUP CREATE`, `GROUP ADD`, and `GROUP REMOVE`.
- [x] Persist role, group, capability, and account policy changes atomically.

### Sessions, recovery, and audit

- [x] Add active-session listing for administrators.
- [x] Add administrator session termination for a selected account or session.
- [x] Enforce idle timeout and maximum session lifetime.
- [x] Bind sessions to the authenticated identity, terminal, node, and revocation epoch.
- [x] Reject replayed, expired, malformed, or cross-node login responses.
- [x] Add safe credential-loss recovery requiring a trusted recovery key or physical recovery action.
- [x] Prevent recovery from silently bypassing normal authorization policy.
- [x] Audit login success, login failure, logout, timeout, lockout, recovery, and account changes.
- [x] Redact credentials, challenges, tokens, and private account data from logs and crash reports.
- [x] Add administrator-visible audit queries for account and session activity.

### Verification and documentation

- [ ] Add end-to-end tests for first boot, first login, normal login, logout, and relogin.
- [ ] Add tests for wrong credentials, rate limits, lockouts, expiry, and revocation.
- [ ] Add tests for account creation, deletion, disablement, rename, and last-admin protection.
- [ ] Add tests for credential enrollment, removal, rotation, and credential loss recovery.
- [ ] Add persistence and power-loss tests for account database updates.
- [ ] Add QEMU coverage for the interactive login flow.
- [ ] Document first boot and first administrator setup.
- [ ] Document local and remote login methods.
- [ ] Document account, group, role, credential, session, and recovery commands.
- [ ] Document the emergency recovery process and its security limits.
