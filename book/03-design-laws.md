# 3. The Core Design Laws

SynOS has many features, but a small set of laws repeats everywhere.

## Law 1: authority is data

A process does not get “access because it called the right function.” It gets a capability object with a rights mask. A capability has an owner, object identity, generation, parent link, and possibly a backing physical range.

```text
parent: ALL
   |
   +-- child: READ | MAP
   +-- child: READ
```

Delegation can only reduce rights. A child cannot turn `READ` into `WRITE`. Revocation walks the derivation tree so descendants become unusable.

Easy memory rule: **a capability is a key, not a wish**.

## Law 2: every boundary is bounded

Fixed arrays and explicit limits appear in the kernel, services, shell, scripts, Wasm, network queues, storage pools, and test harness. Bounded does not mean weak. It means the system can answer “how much?” before doing work.

Typical limits include:

- maximum capabilities;
- fixed thread slots;
- queue capacity;
- packet and message sizes;
- filesystem block counts;
- script instruction and recursion budgets;
- Wasm fuel and memory;
- cluster member and federation counts;
- inference cache and journal sizes.

The usual failure is not a hidden allocation. It is a stable status such as `NO_SPACE`, `BUSY`, invalid input, or access denied.

## Law 3: shared bytes beat copied bytes

Large payloads use shared pages and descriptors. The kernel checks ownership and bounds, then services exchange references to the bytes. This supports:

- IPC buffers;
- network packets;
- filesystem data;
- media planes;
- tensors;
- actor mailboxes;
- remote DSM pages.

The descriptor is part of the security boundary. It says where the bytes live, how large they are, and whether the operation reads or writes them.

## Law 4: immutable state makes recovery easier

SynFS uses Copy-on-Write generations. Snapshots pin old roots. Package roots are content-addressed. A backup reads a stable checkpoint. An update stages a new root before activation. Agent sandboxes publish only after validation.

The simple pattern is:

```text
old root -> stage changes -> validate -> publish new root
                    |
                    +-> failure: discard stage, keep old root
```

## Law 5: epochs defeat stale actors

Capabilities, cluster membership, DLM locks, DSM leases, sockets, and handles use generations or epochs. A stale holder may still have bytes that look valid, but its epoch no longer matches the owner’s current state.

```text
node epoch 41: lease accepted
node fenced -> epoch 42
old request from epoch 41: rejected
```

If a feature can outlive a process, node, connection, or reboot, give it an epoch.

## Law 6: errors are part of the API

The project uses structured status values rather than scattered strings. A status carries facility, code, severity, flags, and the OpenVMS odd-value success convention. Shell, services, SDKs, and clients can preserve the same condition meaning.

The important question is not “did it fail?” but “what stable fact should the next layer know?”

## Law 7: evidence is a product feature

Tests have stable IDs. QEMU saves serial logs. Fuzz failures become regression tests. Release gates check image provenance. Platform qualification checks boot, inventory, fabric, migration, and failover evidence.

If an operation cannot explain what happened, it is not finished.

