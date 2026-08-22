# 5. Tasks, Scheduling, IPC, and Capabilities

GhostOS calls a running unit a task or thread depending on the layer. The kernel keeps the representation fixed and generation-checked.

## Tasks and address spaces

The task model distinguishes kernel and user execution modes. Thread contexts contain the registers and scheduling state needed to resume a task. Handles carry generations so a recycled slot cannot be mistaken for the old thread.

The kernel’s fixed thread table has 64 generation-checked slots. This is deliberately small for a bootstrap kernel and deliberately explicit for tests.

## Scheduler

The scheduler supports:

- cooperative threads that run until they yield or block;
- fixed-priority real-time threads;
- earliest-deadline selection among equal priority classes;
- timer-driven preemption of lower-ranked work;
- blocked and woken states;
- cancellation and CPU partitioning policies.

The selection rule is easy to remember:

```text
ready first, higher priority first, earlier deadline next, stable ID last
```

The final stable tie-breaker matters. It keeps deterministic tests and replay from depending on hash order.

## IPC

The kernel IPC queue is a bounded, non-blocking MPMC structure. Small control words travel in the message. Large payloads stay in shared mapped regions described by descriptors.

An IPC operation usually has this shape:

```text
submit(request, shared_buffer_capability)
        |
        v
validate caller + rights + descriptor bounds
        |
        v
service performs bounded work
        |
        v
complete(status, result metadata)
```

There is no hidden “send arbitrary pointer” operation. The receiver gets a capability-mapped region and a declared direction.

## Capabilities

The kernel capability space holds generation-checked tokens. A capability object may represent:

- untyped physical memory;
- a shared memory region;
- an address space;
- a thread;
- system control;
- an IPC channel;
- a distributed resource;
- a logical namespace.

Rights include `READ`, `WRITE`, `EXECUTE`, `MAP`, `CREATE`, `SEND`, `RECEIVE`, `DELEGATE`, `REVOKE`, and `CONTROL`.

## Delegation example

An administrator has a storage service capability:

```text
admin: READ | WRITE | MAP | CREATE | DELEGATE
  -> service: READ | WRITE | MAP
      -> worker: READ
```

The worker can read. It cannot create, map, delegate, or write. If the service is revoked, the worker’s descendant handle becomes invalid too.

## Quotas

Capabilities can carry quota policy. Resources include memory bytes, CPU time, packet bytes, storage bytes, and other service-defined units. A quota decision can be:

- allowed;
- throttled until a timestamp;
- rejected.

The kernel charges before work and refunds when allocation or submission fails. This prevents a failed operation from silently consuming budget.

## Distributed locks and fencing

The kernel DLM supports resource names, lock modes, queues, lease epochs, and node fencing. A failed node is not immediately allowed to have its locks reassigned. Fencing first changes the membership epoch and confirms the node’s NIC/CXL isolation. Only then may ownership move.

## Easy example: a safe service call

Imagine a file write:

```text
1. shell holds WRITE on /data
2. shell maps a 4 KiB shared buffer
3. shell sends {path, offset, len, buffer_cap}
4. fsd rejects len > buffer size
5. fsd writes into a new GhostFS generation
6. fsd returns {status = SUCCESS, generation = 18}
```

Every step gives a different safety property: authority, bounds, validation, immutability, and evidence.

