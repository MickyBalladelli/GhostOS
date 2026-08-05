# Appendix C. Glossary and Mental Models

| Term | Meaning |
| --- | --- |
| ABI | The stable calling/data contract between a caller and a service or kernel |
| Address space | A protected view of virtual memory owned by a task/process |
| Capability | A generation-checked authority token for an object and rights |
| CXL | Compute Express Link memory/device fabric |
| CoW | Copy-on-Write; publish new state while preserving the old state |
| DLM | Distributed Lock Manager for resources, ranges, leases, and epochs |
| DSM | Distributed Shared Memory page movement and coherence |
| Epoch | A generation that makes stale holders or messages invalid |
| Facility | The subsystem that owns a status code |
| GOP | UEFI Graphics Output Protocol framebuffer interface |
| HDM | CXL Host-managed Device Memory decoder mapping |
| HLT | CPU wait instruction; in the VM it is not process exit |
| IPC | Inter-process communication through bounded messages and shared pages |
| L2 memory | Remote Layer-2 memory reached through a transport rather than local mapping |
| LVT | Local APIC local vector table |
| MMIO | Memory-mapped device I/O |
| MPMC | Multi-producer, multi-consumer queue |
| Persona | Kernel-owned execution identity and initial authority set |
| Ring 0 | Privileged kernel execution |
| Ring 3 | Isolated user/service execution |
| SynFS | SynOS Copy-on-Write, fixed-capacity filesystem core |
| SynFS generation | A published immutable filesystem root/version |
| TEE | Trusted execution environment |
| VM | The Rust virtual machine used to boot and test SynOS |

## Six useful diagrams in words

### Capability

```text
owner -> object -> rights -> generation -> audit
```

### IPC

```text
request -> validate -> shared bytes -> complete(status)
```

### Filesystem

```text
old root -> stage -> checksum -> publish
```

### Device lifecycle

```text
online -> drain -> quiesce -> detach
```

### Cluster failure

```text
heartbeat miss -> epoch change -> fence -> reconcile -> rejoin
```

### Agent tool use

```text
schema -> exact grant -> private stage -> approve/discard
```

## The one-sentence design test

For any new operation, ask:

> Who owns the bytes, who owns the authority, what is the bound, what is the epoch, and how do we recover?

If the design answers all five, it is probably speaking SynOS.

