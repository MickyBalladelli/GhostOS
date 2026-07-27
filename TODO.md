# TODO.md: SynOS Project Roadmap

A comprehensive architectural roadmap for building **SynOS**—an active-active, capability-based microkernel designed to overcome legacy Linux limitations, incorporate key OpenVMS concepts, and pool memory across both modern CXL fabrics and legacy multi-PC hardware.

---

## 1. Core SynOS Kernel Architecture & SynFS (Phase 1)
- [ ] **Establish Bare-Metal Bootstrapping**
  - [ ] Implement UEFI and Legacy BIOS (MBR) handoff logic in Rust (`#![no_std]`).
  - [ ] Set up early boot allocators, CPU interrupt handlers, and serial/VGA logging (`_start` entrypoint).
  - [ ] Configure hardware-level page tables (x86_64 `CR3` / ARM `TTBR0` / RISC-V `satp`).
- [ ] **Build Minimal Microkernel Core (<15k LOC)**
  - [ ] Keep the SynOS core strictly in Ring 0; move all drivers, file systems, and network stacks to user space (Ring 3).
  - [ ] Write asynchronous, zero-copy Inter-Process Communication (IPC) primitives for ultra-low latency messaging.
  - [ ] Implement cooperative and real-time thread scheduling primitives.
- [ ] **Native Versioned Filesystem (SynFS - Day 1 Core)**
  - [ ] Implement Copy-on-Write (CoW) B-tree block structures for instant, zero-cost file versioning (`file.txt;1`, `file.txt;2`).
  - [ ] Build OpenVMS-style version resolution into the VFS lookup pipeline (`path/to/file;version`).
  - [ ] Design background block-sharing and retention garbage collection (`synfs_purged`).
- [ ] **Implement Capability-Based Security**
  - [ ] Replace POSIX root permissions with unforgeable, fine-grained object capability tokens.
  - [ ] Build capability delegation models for memory mapping, process creation, and IPC channel authorization.

---

## 2. Legacy PC Hardware, Dual-Boot & Storage Co-Existence
- [ ] **Legacy x86 PC Drivers**
  - [ ] Add basic PCI bus enumeration and generic AHCI/NVMe storage drivers.
  - [ ] Write driver hooks for standard Intel/Realtek Gigabit Ethernet NICs for bare-metal multi-PC networking.
  - [ ] Implement fallback framebuffer display outputs (VGA/VESA/GOP).
- [ ] **Dual-Boot & Storage Co-Existence**
  - [ ] Implement UEFI chainloading (`synos_loader.efi`) for Windows Boot Manager and GRUB.
  - [ ] Support loopback image booting (`synos.img`) directly from NTFS/EXT4 partitions without repartitioning.
  - [ ] Add read-only EXT4 and FAT32/NTFS drivers to access local LLM weights from host OS partitions.

---

## 3. Solving Core Linux Pain Points in SynOS
- [ ] **Enforce Memory Safety**
  - [ ] Build the entire kernel and core system services in Rust/Zig, restricting `unsafe` code to isolated register/page-table blocks.
- [ ] **Replace Legacy Filesystem & Global State Layouts**
  - [ ] Abandon `/etc`, `/usr`, and `/var` directory hierarchies in favor of an immutable, declarative root filesystem in SynFS.
  - [ ] Implement hermetic, content-addressed package isolation (Nix/Flatpak model).
- [ ] **Redesign System I/O & Media Subsystems**
  - [ ] Standardize on an async-first execution model rather than stacking legacy blocking POSIX calls and `io_uring`.
  - [ ] Create unified, zero-copy audio/video pipeline APIs directly in the platform layer.

---

## 4. OpenVMS Feature Integration
- [ ] **Native Distributed Lock Manager (DLM)**
  - [ ] Build SynOS kernel-managed lock mechanisms for shared memory sections, files, and resources across cluster nodes.
- [ ] **Logical Name Tables**
  - [ ] Implement a system-wide, scoped dictionary mapping logical aliases to files, devices, or IPC channels with strict ACLs.
- [ ] **Record Management Services (RMS)**
  - [ ] Add native OS support for structured record types (indexed/sequential) directly within SynFS alongside flat byte streams.
- [ ] **Standardized Command Interface**
  - [ ] Build a CLI dictionary that validates typed arguments and options *before* execution.
  - [ ] Standardize system tool outputs on structured data objects instead of unstructured raw text.
- [ ] **Uniform Error Condition Signals**
  - [ ] Implement a unified 32-bit `$STATUS` code standard across system calls, drivers, and user applications.

---

## 5. Hardware Fabric & Clustering Modes
- [ ] **CXL 3.0 / 3.1 Hardware Fabric (Modern Target)**
  - [ ] Write CXL component register (HDM) drivers to discover and map Type-3 memory devices on boot.
  - [ ] Implement hardware-assisted dynamic memory lease allocation.
- [ ] **Software-Defined Distributed Shared Memory (Legacy Target)**
  - [ ] Implement `#PF` (Page Fault) traps to fetch memory pages over raw layer-2 Ethernet packets between legacy PCs.
  - [ ] Enforce page-level cache coherence across PCs using software DLM lease locks.
- [ ] **Global Address Space & Memory Pooling**
  - [ ] Expose all cluster RAM and VRAM as a single, 64-bit unified address space regardless of hardware transport layer.
  - [ ] Implement background memory page migration based on access patterns and network latency metrics.
- [ ] **Active-Active Fault Tolerance**
  - [ ] Build sub-millisecond heartbeat monitors over network interfaces.
  - [ ] Support transparent page redirection to mirrored memory nodes on physical hardware failure.

---

## 6. Large Language Model (LLM) Enablement
- [ ] **Single-Node Execution Paradigm**
  - [ ] Enable framework-free, multi-terabyte memory allocations on SynOS without manual Tensor/Pipeline parallelism code.
- [ ] **Cluster-Wide Dynamic KV-Cache Pooling**
  - [ ] Allow real-time KV-cache allocation across remote CXL nodes or networked PCs during long-context inference runs.
- [ ] **Zero-Downtime Failover**
  - [ ] Guarantee inference request persistence during node drops through transparent memory degradation handles.

---

## 7. Target Platforms & Emulation
- [ ] **Legacy Hardware Testbed**
  - [ ] Test bare-metal two-node Ethernet clustering using standard consumer PCs.
- [ ] **Local Emulation Sandbox**
  - [ ] Configure multi-instance QEMU/KVM environments using emulated CXL devices (`ivshmem`) for local development.
- [ ] **Enterprise Hardware Targets**
  - [ ] Validate SynOS on rack-scale CXL switched nodes and PCIe/NVLink fabric AI clusters.

---

## 8. Architectural Risk Mitigations & Optimizations
- [ ] **Software DSM & Latency Mitigation**
  - [ ] Implement a predictive asynchronous prefetching engine at the memory allocator layer to prevent CPU stalls during continuous sequential memory reads.
  - [ ] Decouple transport layers by mapping CXL HDM for cache-coherent RAM while treating Layer-2 Ethernet nodes as a tiered NUMA page cache (block-based remote paging).
- [ ] **User-Space RMS Performance Optimization**
  - [ ] Implement lock-free circular buffer IPC Shared-Memory Rings between user-space applications and system daemons to eliminate context switches.
  - [ ] Support direct memory-mapped CoW B-tree node reads via capability handles, moving RMS parsing logic into an in-process runtime library.
- [ ] **Distributed Lock Manager (DLM) Consistency & Thrashing Controls**
  - [ ] Implement granularity-aware leases combining coarse-grained object locks with fine-grained byte-range locks to prevent false sharing on 4KB pages.
  - [ ] Enforce an Epoch-based Read-Copy-Update (RCU) model for read-heavy distributed states (e.g., Logical Name Tables, shared weight matrices).
- [ ] **Bootstrap Heap-Free Capability Management**
  - [ ] Adopt an seL4-style static capability model where physical memory is initially passed to user-space managers as untyped memory capability tokens without dynamic kernel heap allocation.
  - [ ] Embed Capability Derivation Tree (CDT) node pointers directly inside resource descriptor memory pages.
- [ ] **Logical Name Fast-Path Resolution**
  - [ ] Store process-local and system-wide logical name tables in lock-free atomic hash tables residing in read-only shared memory pages for fast user-space alias resolution without Ring 0 switches.

---

## 9. Authentication, Authorization & Identity Services
- [ ] **Ring 3 Identity Daemon (`synos-authd`)**
  - [ ] Implement user-space identity management for initial credential verification (Passkeys, TPM 2.0, SSH keys) bypassing legacy PAM/shadow architectures.
  - [ ] Issue root capability tokens during session instantiation, passing initial capability sets to the login process.
  - [ ] Integrate lightweight `SYSUAF.DAT`-style authorization databases for local and node-local multi-user credential storage.
- [ ] **Capability-Based Object Authorization**
  - [ ] Eliminate root (UID 0) and global ambient authority in favor of unforgeable seL4-style capability tokens for all resources.
  - [ ] Implement capability delegation and attenuation semantics over zero-copy IPC (e.g., stripping write/execute capabilities before handing off handles).
  - [ ] Support transparent, microkernel-enforced capability revocation via derivation tree tracking.
- [ ] **OpenVMS Rights Identifiers & Personas**
  - [ ] Implement dynamic Rights Identifiers (e.g., `LLM_OPERATOR`, `NETWORK_INBOUND`, `BATCH_JOB`) assigned to active process execution contexts.
  - [ ] Build capability dropping system calls allowing processes to dynamically remove or suspend active rights identifiers (`SET RIGHTS_LIST/DISABLE`) before executing untrusted code.
  - [ ] Implement Scoped Logical Name Table access controls (`PROCESS`, `JOB`, `GROUP`, `SYSTEM`) backed by Capability ACLs.
- [ ] **Distributed Multi-Node Authorization**
  - [ ] Support cross-node capability delegation over CXL 3.0 fabrics and Layer-2 Ethernet using cryptographic capability tokens (e.g., Macaroons / Amoeba capabilities).
  - [ ] Integrate with Distributed Lock Manager (DLM) to enforce cluster-wide lease controls and prevent unauthorized remote page faults (#PF) on Software DSM targets.
- [ ] **Owner-Delegated Resource Lending**
    - [ ] Build capability attenuation primitives enabling node owners to issue restricted, time-bound memory/compute tokens to remote cluster users.
    - [ ] Implement transparent microkernel revocation hooks allowing resource providers to reclaim remote-mapped RAM/VRAM instantly.


---

## 10. Multi-Cluster Federation & Cross-Cluster Sandboxing
- [ ] **Inter-Cluster Capability Exchanges ("Cluster of Clusters")**
  - [ ] Implement inter-cluster cryptographic discovery protocols to federate distinct SynOS clusters without centralized management plane dependencies.
  - [ ] Build multi-cluster resource trading primitives allowing Cluster A to lease idle CPU/RAM/VRAM capacity from Cluster B.
- [ ] **Zero-Knowledge Micro-Silo Sandboxing**
  - [ ] Enforce strict "Blind Sandbox" isolation scopes for cross-cluster workloads: tenant processes on borrowed nodes cannot inspect host process trees, local SynFS mountpoints, or host network sockets.
  - [ ] Leverage CXL-IDE and CPU hardware isolation (e.g., AMD SEV / Intel TDX / ARM CCA) where available to encrypt borrowed memory frames in-transit and at-rest.
- [ ] **Cross-Cluster Lease Arbitration & Preemption**
  - [ ] Implement sub-millisecond inter-cluster revocation signals to allow lending clusters to reclaim borrowed hardware instantly when local priority workloads wake up.
  - [ ] Extend the Distributed Lock Manager (DLM) with cross-cluster epoch fencing to prevent stale reads or split-brain states when inter-cluster leases expire.

┌─────────────────────────┐               ┌─────────────────────────┐
│       CLUSTER A         │               │       CLUSTER B         │
│  (Running Heavy App)    │               │     (Idle Worker)       │
│                         │               │                         │
│  [App A Task] ──────────┼──(Presents)──►│  [Isolated Micro-Silo]  │
│                         │  Capability   │   • 128GB Shared RAM    │
│                         │   Token       │   • 16 CPU Cores        │
│                         │               │   • ZERO OS / App Visibility
└─────────────────────────┘               └─────────────────────────┘


---

## 11. System Diagnostics, Observability & Auditing
- [ ] **Bare-Metal Boot Logging (Phase 1)**
  - [ ] Implement early-boot raw serial port (COM1/16550 UART) and VGA framebuffer fallback writers (`#![no_std]`).
  - [ ] Build a lock-free, zero-allocation ring-buffer queue for early kernel initialization traces before memory allocators online.
- [ ] **Structured Trace Subsystem (Ring 0 / Native)**
  - [ ] Create a zero-allocation structured tracing engine (`trace!`, `info!`, `warn!`, `error!`) passing typed event payloads instead of formatted string buffers.
  - [ ] Assign unique 128-bit correlation IDs to asynchronous IPC messages and remote memory accesses for distributed tracing across CXL and Ethernet nodes.
- [ ] **Ring 3 Log & Audit Daemon (`synos-logd`)**
  - [ ] Implement a user-space logging daemon consuming kernel ring buffers via zero-copy IPC shared memory pages.
  - [ ] Stream structured logs to SynFS binary journal streams (`SYS$LOG:SYSTEM.JOURNAL;1`) with automated background CoW retention rotation.
  - [ ] Add an OpenVMS-style Operator Communication Manager (OPCOM) interface allowing real-time terminal broadcasts for critical system alarms.
- [ ] **Security Auditing & Audit Analysis Utility**
  - [ ] Build a dedicated, immutable security audit pipeline (`$AUDIT_EVENT`) recording capability grants, revocations, and authentication checks.
  - [ ] Create a structured log query utility (`analyze/audit` CLI tool) to filter binary system traces by time window, capability handle, cluster node ID, or error status.
  


┌─────────────────────────────────────────────────────────────┐
│                       Ring 0 Kernel                         │
│  Kernel Tracing Macros (trace!, info!) ──> Ring-Buffer Queue│
└──────────────────────────────┬──────────────────────────────┘
                               │ Zero-Copy IPC / Mapped Buffer
                               ▼
┌─────────────────────────────────────────────────────────────┐
│                   Ring 3 Logging Daemon                     │
│               (`synos-logd` / OpenVMS OPCOM)                │
│                                                             │
│ ┌────────────────────────┐       ┌────────────────────────┐ │
│ │ Local Storage Writer   │       │ Cluster Audit Network  │ │
│ │ (Binary SynFS Stream)  │       │ (Distributed Trace Stream)│
│ └────────────────────────┘       └────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘



---

## 12. Rust Toolchain, Runtime & System Ecosystem
- [ ] **Custom Rust Target & `std` Platform Layer**
  - [ ] Define the `x86_64-unknown-synos` and `aarch64-unknown-synos` target specifications.
  - [ ] Implement a native `std::sys::synos` backend mapping Rust primitives directly to SynOS capabilities, zero-copy IPC, and SynFS.
- [ ] **Zero-Copy IPC Crate (`synos-ipc`)**
  - [ ] Build a high-performance IPC library using `zerocopy`/`rkyv` for zero-allocation structured message passing between Ring 3 daemons and Ring 0.
- [ ] **C / FFI Compatibility Layer**
  - [ ] Provide an optional `synos-posix-compat` crate for running legacy C/C++ code (e.g., C-based LLM backends) via light syscall translation.

---

## 13. Native Rust Interactive Shell (`syn-shell`)
- [ ] **Async Command Interpreter**
  - [ ] Build an interactive CLI with OpenVMS DCL-inspired syntax, type-safe argument validation, and structured data outputs.
  - [ ] Implement system diagnostics tools (`SHOW MEMORY/CLUSTER`, `SHOW PROCESS`, `MONITOR`).
- [ ] **Batch & Job Management**
  - [ ] Build a system-wide task queue service for background processing and automated pipeline runs.

---

## 14. User-Space Async Networking
- [ ] **Ring 3 Network Daemon (`synos-netd`)**
  - [ ] Build a pure-Rust user-space TCP/IP stack (`smoltcp`-backed) with zero-copy packet queues.
  - [ ] Expose capability-authenticated sockets via IPC shared-memory ring buffers.

---

## 15. Service Isolation & Fault Recovery
- [ ] **Supervisor Service (`synos-init`)**
  - [ ] Implement dynamic driver recovery in Rust: catch panics/crashes in Ring 3 storage or network drivers and restart them without disrupting other services.
- [ ] **Cluster Panic & Node Isolation**
  - [ ] Implement eviction and fencing logic in the Distributed Lock Manager (DLM) to isolate dropped nodes safely during software DSM memory operations.


---

## 16. Pure-Rust AI Compute Engine & GPU Abstraction
- [ ] **Native ML Framework Bindings (`synos-compute`)**
  - [ ] Port pure-Rust ML runtimes (Candle / Burn) to target `std::sys::synos` natively.
  - [ ] Implement direct zero-copy tensor mapping between SynOS IPC memory pages and compute runtimes.
- [ ] **User-Space Accelerator Interfaces**
  - [ ] Build Ring 3 PCIe/Vulkan driver abstractions for GPU/NPU compute offloading.

---

## 17. Rust Ecosystem Toolchain & Package Distribution
- [ ] **Cargo Extension (`cargo-synos`)**
  - [ ] Build toolchain utilities for automated cross-compiling, manifest signing, and binary bundle generation.
- [ ] **Hermetic Package Daemon (`synos-pkg`)**
  - [ ] Implement content-addressed package management backing declarative system configurations on SynFS.

---

## 18. Volume Management & Disaster Recovery
- [ ] **Storage Pool Administration**
  - [ ] Implement SynFS storage pool management for NVMe, CXL persistent memory, and network blocks.
- [ ] **OpenVMS-Inspired Backup Tool (`synos-backup`)**
  - [ ] Implement volume checkpointing and background streaming using SynFS Copy-on-Write snapshot trees.

---

## 19. Power & Hardware Lifecycle Management
- [ ] **ACPI & Thermal Subsystem**
  - [ ] Integrate pure-Rust ACPI parsing for hardware event handling, thermal throttling, and power state control (shutdown/reboot).
- [ ] **Dynamic Hot-Plug Support**
  - [ ] Support on-the-fly CXL memory module and NVMe storage insertion/removal events.

---

## 20. Cluster Topology Visualization
- [ ] **Visual System & Fabric Monitor (`synos-top`)**
  - [ ] Build a terminal/framebuffer dashboard visualizing cluster RAM/VRAM heatmaps, remote DSM page fault latency, and dynamic capability graphs in real time.


---

## 21. Cross-Platform Client SDKs & Frontend Gateways
- [ ] **Multi-Platform Rust Client SDK (`synos-client-sdk`)**
  - [ ] Build a cross-platform library (supporting macOS, iOS, Android, and WebAssembly) for remote capability exchange and RPCs.
- [ ] **Native Mobile & macOS Control Applications**
  - [ ] Develop native apps (Swift/SwiftUI) for cluster state monitoring, job submission, and capability handle delegation.

---

## 22. Remote Console & Remote Desktop Subsystems
- [ ] **WebAssembly Terminal & SSH Gateway (`synos-webterm`)**
  - [ ] Implement a high-performance WebAssembly/WebGPU terminal frontend (`syn-shell`) and an SSH daemon in Ring 3.
  - [ ] Preserve VT100/VT420/DECterm terminal handling for remote admin access.
- [ ] **Headless Low-Latency Display Streaming (`synos-remote-display`)**
  - [ ] Build a zero-copy WebRTC/AV1 streaming daemon for remoting GUI/dashboard interfaces to phones, tablets, and desktop browsers.

---

## 23. Mobile & Remote Security Gateways
- [ ] **Remote Token Attenuation & Passkey Auth**
  - [ ] Implement remote capability token issuing with strict scope limits for untrusted frontend devices.
  - [ ] Integrate WebAuthn / Passkeys / Device Biometrics into `synos-authd` for remote administrative access.



---

## 24. Native Application Model & Service Runtimes
- [ ] **Declarative Application Manifests (`App.toml`)**
  - [ ] Implement capability-constrained app manifest specifications and supervisor spawning logic.
- [ ] **Distributed Actor Framework (`synos-actors`)**
  - [ ] Build a pure-Rust actor system using native IPC and Software DSM for multi-node process orchestration without manual networking boilerplate.

---

## 25. Application Data Layer & Native RMS
- [ ] **Rust Record Management API (`synos-rms`)**
  - [ ] Build idiomatic Rust bindings for OpenVMS-style indexed (ISAM) files, structured data records, and DLM-backed record-level locking.
- [ ] **Embedded CoW Database Engine**
  - [ ] Implement zero-copy key-value and transaction storage libraries optimized for SynFS B-trees.

---

## 26. High-Level AI Execution & Agent Pipelines
- [ ] **Native LLM Service Gateway (`synos-inference`)**
  - [ ] Build a user-space OpenAI/gRPC-compatible API server using pooled cluster RAM/VRAM for KV-cache allocation.
- [ ] **Persistent Agent Execution State**
  - [ ] Provide continuous CoW snapshotting for long-running AI agent stacks and execution states.

---

## 27. Web Application & Microservices Layer
- [ ] **Async HTTP / gRPC Stack (`synos-http`)**
  - [ ] Provide lightweight native web server primitives (`axum`/`hyper` ports) bound directly to `synos-netd` and capability checks.


---

## 28. Native Command Scripting (`syn-script`)
- [ ] **Structured Pipeline Engine**
  - [ ] Build a strongly-typed script interpreter passing structured Rust objects through IPC channels instead of raw text streams.
- [ ] **Capability & Logical Name Control**
  - [ ] Implement native syntax for Logical Name manipulation, symbol creation, and capability token attenuation.
- [ ] **OpenVMS DCL-Style Status Handling**
  - [ ] Enforce `$STATUS`-driven error propagation and conditional execution primitives.

---

## 29. Embedded Scripting & Wasm Extension Runtime
- [ ] **Pure-Rust Embedded Engine (Rhai Integration)**
  - [ ] Embed `Rhai` for fast, memory-safe system automation and service scripting without binary re-compilation.
- [ ] **Sandboxed WebAssembly Scripting (`synos-wasm-script`)**
  - [ ] Provide a zero-trust Wasm script engine (`wasmtime`/`wasmi`) for executing untrusted user/agent code with fine-grained capability restrictions.

---

## 30. AI Agent Orchestration & Deterministic Execution
- [ ] **Dry-Run CoW Sandboxing**
  - [ ] Implement isolated CoW execution environments for AI-generated scripts to validate system operations before committing changes.
- [ ] **Structured LLM Function Reflection**
  - [ ] Automatically export system script command signatures as structured tool-calling schemas for AI agents.


