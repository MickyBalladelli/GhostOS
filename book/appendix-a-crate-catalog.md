# Appendix A. Crate Catalog

This catalog is a navigation aid. Read the crate’s `Cargo.toml`, `README`, and `src/lib.rs` for exact APIs.

## Boot and kernel

| Package | What it owns |
| --- | --- |
| `ghostos-boot-protocol` | Versioned `BootInfo`, memory regions, framebuffer, and handoff types |
| `ghostos-uefi` | UEFI application, PE/COFF loading, memory map, boot services exit, chainload |
| `ghostos-kernel` | no_std Ring 0: allocator, tasks, scheduler, IPC, capabilities, DLM, page faults, console, power, runtime, shell entry |
| `ghostos-status` | Shared condition/status encoding and OpenVMS-style success semantics |
| `ghostos-runtime` | User ABI, descriptor validation, filesystem and service calls |
| `ghostos-posix-compat` | Bounded POSIX/Linux compatibility surface over GhostOS services |
| `ghostos-ipc` | Shared IPC structures and bounded transport primitives |
| `cargo-ghostos` | Cargo workflow integration and compiler commands |
| `ghostos-compiler` | Native Rust compiler driver for GhostOS targets |
| `ghostos-rustd` | Bounded compiler-service requests, jobs, policy, and cache keys |

## Filesystem, storage, and packages

| Package | What it owns |
| --- | --- |
| `ghostos-ghostfs` | Fixed-capacity CoW filesystem core, versions, B+trees, blocks, checkpoints |
| `ghostos-fsd` | Filesystem daemon, namespace and protocol boundary |
| `ghostos-host-filesystems` | Read-only Ext4, FAT32, and NTFS discovery |
| `ghostos-rms` | Sequential/indexed records, DLM record locking, embedded key/value database |
| `ghostos-backup` | Checkpointed, checksummed GhostFS backup worker |
| `ghostos-durability` | Shared bounded durability and recovery primitives |
| `ghostos-pkg` | Content-addressed packages, dependency resolution, signatures, rollback inputs |
| `ghostos-system-model` | Sealed roots, logical names, ACLs, package activation metadata |
| `ghostos-kvd` | Native bounded key-value cache |
| `ghostos-storaged` | Capability-gated enterprise remote storage models |
| `ghostos-path-pattern` | Bounded path and wildcard matching |

## Hardware and platform

| Package | What it owns |
| --- | --- |
| `ghostos-platform-io` | Fixed asynchronous request/completion queues and buffer descriptors |
| `ghostos-legacy-pc-drivers` | PCI, AHCI, NVMe, E1000, RTL8169, DMA building blocks |
| `ghostos-power` | ACPI parsing, thermal policy, reset, shutdown, reboot |
| `ghostos-ras` | Reliability, availability, serviceability records and fault response |
| `ghostos-fabric` | CXL discovery, HDM decoders, leases, global memory map, DSM |
| `ghostos-time-sync` | Monotonic/synchronized clock and skew contracts |

## Network and clients

| Package | What it owns |
| --- | --- |
| `ghostos-netd` | Heap-free TCP/IP service, socket capabilities, packet slots |
| `ghostos-http` | HTTP/1, router, response encoding, gRPC framing and server primitives |
| `ghostos-protocol` | Shared transport guards, replay windows, and traffic classes |
| `ghostos-client-sdk` | Portable capability and frontend RPC client model |
| `ghostos-webterm` | Remote terminal protocol and session model |
| `ghostos-remote-display` | Remote console/display transport |

## Shell, scripting, and applications

| Package | What it owns |
| --- | --- |
| `ghostos-shell` | Typed shell parser, editor, commands, structured output, cluster admin |
| `ghostos-script` | Native DCL-style scripts, conditions, pipelines, typed tools, capability use |
| `ghostos-embedded-script` | Capability-scoped Rhai automation |
| `ghostos-wasm-script` | Zero-trust Wasmi runtime with fuel and memory limits |
| `ghostos-app` | Application manifest and capability admission |
| `ghostos-actors` | Local/distributed actors, mailboxes, supervisors, replies |
| `ghostos-init` | Service supervision and bounded restart |
| `ghostos-balancerd` | Active-active actor placement and cache-aware lease coordination |

## Security and operations

| Package | What it owns |
| --- | --- |
| `ghostos-auth` | Identity, passkeys, TPM/SSH credentials, sessions, personas |
| `ghostos-auditd` | Package and security audit service |
| `ghostos-shield` | Runtime protection, quarantine, hardware admission |
| `ghostos-confidential` | Attestation, confidential fabric transport, enclave admission |
| `ghostos-observability` | Bounded events, fields, levels, metrics, traces |
| `ghostos-logd` | Log sinks, filtering, flush, rotation, restart |
| `ghostos-inspect` | Capability-scoped inspection and diagnostics |
| `ghostos-debug` | Debug probes, GDB, coredumps, safe tracing |
| `ghostos-top` | Terminal/framebuffer topology and health dashboard |
| `ghostos-replay` | Deterministic input logs, checkpoints, flight recorders |
| `ghostos-heal` | CoW daemon recovery and health monitoring |
| `ghostos-update` | Signed staged updates, compatibility, activation, rollback |
| `ghostos-declarative` | Signed declarative system configuration and atomic activation |

## AI and scale

| Package | What it owns |
| --- | --- |
| `ghostos-compute` | Capability-mapped tensors, accelerator queues, GPU/NPU boundary |
| `ghostos-llm` | Model allocation, CXL memory, KV caches, mirrored checkpoints |
| `ghostos-inference` | OpenAI-compatible and gRPC inference service, token recovery |
| `ghostos-agentd` | Semantic memory and context bus |
| `ghostos-agent-bridge` | Typed agent tools, short-lived grants, sandbox publish/discard |
| `ghostos-mesh` | Edge-to-cloud cluster mesh and offload |

## Test machine and support

| Package | What it owns |
| --- | --- |
| `ghostos-vm` | x86_64 VM, firmware, devices, disks, snapshots, terminal, cluster fixtures |
| `ghostos-hello-world` | Minimal cross-target example application |
| `ghostos-test-support` | Deterministic fixtures, fake devices, fault injection, cleanup, golden data |
| `ghostos-fuzz` | LibFuzzer targets for parser, filesystem, HTTP, script, VM decoder/device/image paths |
