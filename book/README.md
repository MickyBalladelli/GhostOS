# SynOS: A Book of the System

## A practical guide to the kernel, services, filesystem, clusters, AI, and virtual machine

> SynOS is a capability-based, `no_std` operating-system project written in Rust. Its small kernel owns the hard boundaries. User-space services own drivers, storage, networking, policy, applications, and AI. The virtual machine makes the whole system testable.

This book explains the repository as it exists today. It is written for a new developer who wants a map before touching code, and for an experienced developer who wants one reference for the system’s contracts.

The project is large. It contains a boot chain, a microkernel core, a filesystem, storage and network services, OpenVMS-inspired interfaces, a cluster fabric, security services, AI runtimes, clients, a QEMU-backed virtual machine, tests, fuzzers, and release tooling. The book groups those pieces by the problems they solve.

## The short version

Remember five words:

**Boot. Boundaries. Bytes. Borrowing. Recovery.**

1. **Boot** gives the kernel a versioned `BootInfo` contract.
2. **Boundaries** keep drivers, filesystems, and policies outside Ring 0.
3. **Bytes** move through fixed buffers, shared pages, immutable filesystem versions, and bounded queues.
4. **Borrowing** is capability control: a process gets only the rights and resources it was given.
5. **Recovery** is normal behavior: epochs, snapshots, fencing, mirrors, rollback, and replay make failure explicit.

## Table of contents

### Part I — The shape of SynOS

1. [What SynOS is](01-what-synos-is.md)
2. [How the repository is organized](02-repository-map.md)
3. [The core design laws](03-design-laws.md)

### Part II — From firmware to a running system

4. [Boot, memory, and the microkernel](04-boot-and-kernel.md)
5. [Tasks, scheduling, IPC, and capabilities](05-tasks-ipc-capabilities.md)
6. [The status and ABI contracts](06-status-and-abi.md)

### Part III — Data and devices

7. [SynFS, persistence, and storage](07-synfs-storage.md)
8. [Drivers, platform I/O, power, and media](08-drivers-platform-media.md)
9. [Networking, HTTP, and remote surfaces](09-networking-and-web.md)

### Part IV — The human and application layer

10. [The shell and OpenVMS ideas](10-shell-and-openvms.md)
11. [Scripts, Wasm, applications, jobs, and actors](11-scripting-apps-actors.md)
12. [Identity, security, observability, and recovery](12-security-observability-recovery.md)

### Part V — Scale and intelligence

13. [Fabric, clustering, federation, and time](13-fabric-clusters-federation.md)
14. [Compute, LLMs, agents, and semantic memory](14-compute-llm-agents.md)
15. [Declarative systems, updates, and operations](15-operations-and-lifecycle.md)

### Part VI — The virtual machine and developer workflow

16. [The SynOS virtual machine](16-virtual-machine.md)
17. [Build, test, fuzz, and release](17-build-test-release.md)
18. [Cookbook: memorable workflows](18-cookbook.md)

### Appendices

- [A. Crate catalog](appendix-a-crate-catalog.md)
- [B. Feature map](appendix-b-feature-map.md)
- [C. Glossary and mental models](appendix-c-glossary.md)

## How to read this book

If you want to boot something, read Chapters 1, 4, and 16.

If you want to add a service, read Chapters 3, 5, 6, 7, and 17.

If you want to add a shell command, read Chapters 9, 10, and 11.

If you want to work on clusters or AI, read Chapters 12–15 after reading the boundary chapters.

If you want to understand a failing test, start with Chapter 17 and then jump to the subsystem chapter.

## Source of truth

The code is the final authority. The main companion documents are:

- [`README.md`](../README.md) — project introduction and major contracts.
- [`todo/TODO-first.md`](../todo/TODO-first.md) — the numbered feature and
  test-completion plan.
- [`TODO.md`](../TODO.md) — the current hardening and developer-experience
  roadmap.
- [`docs/testing.md`](../docs/testing.md) — test tiers, evidence, and validation rules.
- [`docs/inventory-diagrams.md`](../docs/inventory-diagrams.md) — generated
  source maps for crates, services, protocols, capabilities, formats, and
  tests.
- [`docs/compatibility-matrix.md`](../docs/compatibility-matrix.md) —
  format, protocol, target, and SDK compatibility.
- [`docs/persistence-compatibility.md`](../docs/persistence-compatibility.md) — durable formats, backup, migration, downgrade, and recovery evidence.
- [`docs/system-configuration.md`](../docs/system-configuration.md) — system configuration model.
- [`virtual_machine/README.md`](../virtual_machine/README.md) — VM usage.
- [`todo/TODO-VM-first.md`](../todo/TODO-VM-first.md) — VM-specific roadmap
  and quality gates.

Some examples in this book are teaching examples. They show the shape of an API or workflow and may omit imports, error plumbing, or platform setup. Commands are intended to be copied from the repository root unless the text says otherwise.
