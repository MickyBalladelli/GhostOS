# 1. What GhostOS Is

GhostOS is an operating-system project with a strict split between a small kernel and a wide user space.

The kernel does a few jobs very carefully:

- start the machine;
- manage address spaces, threads, and physical memory;
- deliver interrupts and IPC;
- enforce capabilities and quotas;
- provide the minimum runtime and power path;
- protect the system when a process or node fails.

Everything that can be isolated is meant to live outside Ring 0: device drivers, filesystems, networking, package policy, identity, shell commands, HTTP services, AI services, and cluster control.

This is not only a security choice. It is a testing choice. A driver in user space can be replaced by a fake device. A filesystem can be tested with an in-memory block device. A network service can be driven by a deterministic queue. A cluster can be simulated without a real rack. The same boundary that limits damage also makes behavior easier to replay.

## The system in one picture

```text
 Firmware / BIOS / UEFI
          |
          v
   BootInfo + kernel entry
          |
          v
 +-----------------------+
 | Ring 0 microkernel     |
 | memory | tasks | IPC   |
 | caps   | IRQ  | power |
 +-----------------------+
      |      |       |
      v      v       v
   ghostfs   netd    drivers
      |      |       |
      +------+-------+
             |
             v
 shell | apps | HTTP | AI | cluster | clients
             |
             v
        GhostOS VM / QEMU
```

The arrows are contracts, not casual function calls. A service normally receives a capability, maps a bounded shared region, performs a bounded operation, and returns a status plus a completion record.

## What problem is GhostOS trying to solve?

The project combines several ideas that are often separate:

1. **Microkernel isolation.** A bad service should not own the whole machine.
2. **OpenVMS-style operability.** Commands, conditions, logical names, records, jobs, and locks should be structured and inspectable.
3. **Modern capability security.** Authority should be explicit, attenuable, revocable, generation-checked, and auditable.
4. **Distributed memory and storage.** Local RAM, CXL memory, remote memory, VRAM, NVMe, and network storage should be controlled by the same kinds of leases and failure rules.
5. **AI as a system workload.** Models, KV caches, agents, scripts, tools, and semantic memory need quotas, recovery, and isolation like any other service.
6. **Deterministic engineering.** Tests should not need lucky timing, hidden host paths, or a real cluster to verify core behavior.

## A small story: creating one file

Suppose a user runs:

```text
CREATE /DATA/notes.txt
```

The important path is not “shell calls filesystem.” It is closer to this:

1. `ghostos-shell` parses the command and validates its typed arguments.
2. The shell’s principal must have a filesystem capability with create/write rights.
3. The request crosses IPC to the filesystem service.
4. The service checks the path against its namespace and ACL.
5. GhostFS creates a new immutable metadata generation using Copy-on-Write blocks.
6. The service returns a structured status and record.
7. The shell renders that record as text, list output, or JSON.
8. The audit path can record who created what and which generation changed.

The user sees one command. The system preserves a chain of authority, data ownership, version history, and evidence.

## The most useful mental model

Treat GhostOS as a **graph of bounded state machines**.

- A capability table is a state machine.
- A thread is a state machine.
- A filesystem generation is a state machine.
- A network socket is a state machine.
- A cluster member moves through membership epochs.
- An inference request moves through journaled token states.

Every important transition should answer four questions:

1. Who is allowed to request it?
2. What is the maximum work or memory it can consume?
3. What happens if the operation is interrupted?
4. What evidence proves the final state?

When you design a new feature, answer those questions before writing the happy path.

