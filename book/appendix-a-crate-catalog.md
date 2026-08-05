# Appendix A. Crate Catalog

This catalog is a navigation aid. Read the crate’s `Cargo.toml`, `README`, and `src/lib.rs` for exact APIs.

## Boot and kernel

| Package | What it owns |
| --- | --- |
| `synos-boot-protocol` | Versioned `BootInfo`, memory regions, framebuffer, and handoff types |
| `synos-uefi` | UEFI application, PE/COFF loading, memory map, boot services exit, chainload |
| `synos-kernel` | no_std Ring 0: allocator, tasks, scheduler, IPC, capabilities, DLM, page faults, console, power, runtime, shell entry |
| `synos-status` | Shared condition/status encoding and OpenVMS-style success semantics |
| `synos-runtime` | User ABI, descriptor validation, filesystem and service calls |
| `synos-posix-compat` | Bounded POSIX/Linux compatibility surface over SynOS services |
| `synos-ipc` | Shared IPC structures and bounded transport primitives |
| `cargo-synos` | Cargo workflow integration |

## Filesystem, storage, and packages

| Package | What it owns |
| --- | --- |
| `synos-synfs` | Fixed-capacity CoW filesystem core, versions, B+trees, blocks, checkpoints |
| `synos-fsd` | Filesystem daemon, namespace and protocol boundary |
| `synos-host-filesystems` | Read-only Ext4, FAT32, and NTFS discovery |
| `synos-rms` | Sequential/indexed records, DLM record locking, embedded key/value database |
| `synos-backup` | Checkpointed, checksummed SynFS backup worker |
| `synos-pkg` | Content-addressed packages, dependency resolution, signatures, rollback inputs |
| `synos-system-model` | Sealed roots, logical names, ACLs, package activation metadata |
| `synos-kvd` | Native bounded key-value cache |
| `synos-storaged` | Capability-gated enterprise remote storage models |
| `synos-path-pattern` | Bounded path and wildcard matching |

## Hardware and platform

| Package | What it owns |
| --- | --- |
| `synos-platform-io` | Fixed asynchronous request/completion queues and buffer descriptors |
| `synos-legacy-pc-drivers` | PCI, AHCI, NVMe, E1000, RTL8169, DMA building blocks |
| `synos-power` | ACPI parsing, thermal policy, reset, shutdown, reboot |
| `synos-ras` | Reliability, availability, serviceability records and fault response |
| `synos-fabric` | CXL discovery, HDM decoders, leases, global memory map, DSM |
| `synos-time-sync` | Monotonic/synchronized clock and skew contracts |

## Network and clients

| Package | What it owns |
| --- | --- |
| `synos-netd` | Heap-free TCP/IP service, socket capabilities, packet slots |
| `synos-http` | HTTP/1, router, response encoding, gRPC framing and server primitives |
| `synos-client-sdk` | Portable capability and frontend RPC client model |
| `synos-webterm` | Remote terminal protocol and session model |
| `synos-remote-display` | Remote console/display transport |

## Shell, scripting, and applications

| Package | What it owns |
| --- | --- |
| `syn-shell` | Typed shell parser, editor, commands, structured output, cluster admin |
| `syn-script` | Native DCL-style scripts, conditions, pipelines, typed tools, capability use |
| `synos-embedded-script` | Capability-scoped Rhai automation |
| `synos-wasm-script` | Zero-trust Wasmi runtime with fuel and memory limits |
| `synos-app` | Application manifest and capability admission |
| `synos-actors` | Local/distributed actors, mailboxes, supervisors, replies |
| `synos-init` | Service supervision and bounded restart |
| `synos-balancerd` | Active-active actor placement and cache-aware lease coordination |

## Security and operations

| Package | What it owns |
| --- | --- |
| `synos-auth` | Identity, passkeys, TPM/SSH credentials, sessions, personas |
| `synos-auditd` | Package and security audit service |
| `synos-shield` | Runtime protection, quarantine, hardware admission |
| `synos-confidential` | Attestation, confidential fabric transport, enclave admission |
| `synos-observability` | Bounded events, fields, levels, metrics, traces |
| `synos-logd` | Log sinks, filtering, flush, rotation, restart |
| `synos-inspect` | Capability-scoped inspection and diagnostics |
| `synos-debug` | Debug probes, GDB, coredumps, safe tracing |
| `synos-top` | Terminal/framebuffer topology and health dashboard |
| `synos-replay` | Deterministic input logs, checkpoints, flight recorders |
| `synos-heal` | CoW daemon recovery and health monitoring |
| `synos-update` | Signed staged updates, compatibility, activation, rollback |
| `synos-declarative` | Signed declarative system configuration and atomic activation |

## AI and scale

| Package | What it owns |
| --- | --- |
| `synos-compute` | Capability-mapped tensors, accelerator queues, GPU/NPU boundary |
| `synos-llm` | Model allocation, CXL memory, KV caches, mirrored checkpoints |
| `synos-inference` | OpenAI-compatible and gRPC inference service, token recovery |
| `synos-agentd` | Semantic memory and context bus |
| `synos-agent-bridge` | Typed agent tools, short-lived grants, sandbox publish/discard |
| `synos-mesh` | Edge-to-cloud cluster mesh and offload |

## Test machine and support

| Package | What it owns |
| --- | --- |
| `synos-vm` | x86_64 VM, firmware, devices, disks, snapshots, terminal, cluster fixtures |
| `synos-test-support` | Deterministic fixtures, fake devices, fault injection, cleanup, golden data |
| `synos-fuzz` | LibFuzzer targets for parser, filesystem, HTTP, script, VM decoder/device/image paths |
