# SynOS

SynOS now has an initial `no_std` bare-metal bootstrap for x86_64 BIOS and
UEFI machines, plus early page-table backends for x86_64, AArch64, and
RISC-V 64.

## Boot contract

Both x86 boot paths pass a versioned `BootInfo` pointer to the kernel:

- BIOS stage 1 loads stage 2 through INT 13h extensions.
- BIOS stage 2 enables A20, reads the E820 map, enters long mode, and jumps to
  the kernel `_start`.
- The UEFI loader captures the firmware memory map, exits boot services, and
  calls the same kernel entry.

The kernel starts serial/VGA output, creates an early frame allocator, installs
an identity-mapped supervisor root page table, loads interrupt handlers, and
then enables interrupts. Each Ring 3 process gets a distinct root with the
shared supervisor mapping plus private user virtual mappings.

## Build

Install the Rust targets and LLVM tools listed in `rust-toolchain.toml`, plus
`clang`. The image script uses Rust's bundled linker and object copier.

Build a BIOS disk image:

```sh
./scripts/build-bios-image.sh
```

The image is written to `build/bios/synos-bios.img`.

Run the interactive BIOS VM (builds the release `synos-vm` first if needed):

```sh
cargo build -p synos-vm --release
./start-synos.sh
```

Start named VMs with separate persistent disks so they can run side by side:

```sh
./start-synos.sh vm1
./start-synos.sh vm2
```

Run a temporary copy with `./start-synos.sh --new`. Its disk changes are
discarded when the VM exits.

Writable disks create a `<image>.synos.lock` ownership marker. If start fails
with `disk is already locked` after a crashed or killed VM, inspect and recover
the stale lock:

```sh
./target/release/synos-vm disk lock ./virtual_machine/state/data.raw
./target/release/synos-vm disk recover-lock ./virtual_machine/state/data.raw
```

`recover-lock` only removes the marker when the recorded owner PID is gone.
Then run `./start-synos.sh` again.

Build the BIOS image, build the virtual machine in release mode, and run the
workspace tests in one step:

```sh
./scripts/build-and-test.sh
```

Build the UEFI application:

```sh
./scripts/build-uefi-loader.sh
```

The EFI executable is written under
`target/x86_64-unknown-uefi/release/synos-loader.efi`.

Build a portable loopback image:

```sh
./scripts/build-uefi-loader.sh
./scripts/build-portable-image.sh
```

This needs `dosfstools` and `mtools`. The result is
`build/portable/synos.img`.

The AArch64 backend programs `TTBR0_EL1`; the RISC-V backend programs an Sv39
root through `satp`. Platform-specific firmware entry shims for those machines
can hand their memory map to the same `kernel_entry`.

## Test

Run the host-side unit tests for the default workspace members:

```sh
cargo test
```

The test contract, tiers, environment variables, evidence format, and TODO
feature inventory are in [`docs/testing.md`](docs/testing.md).

The bare-metal kernel and UEFI entry binaries are not test harnesses. Their
reusable logic is tested through the kernel library with host-safe hardware
stubs.

## Documentation

The [system book](book/README.md) gives the repository map and design guide.
The [source inventory diagrams](docs/inventory-diagrams.md), [compatibility
matrix](docs/compatibility-matrix.md), and [test contract](docs/testing.md)
are checked against source metadata and roadmap files by:

```sh
python3 scripts/validate-documentation.py
```

Bare-metal controller, disk, USB, and input limits are listed in the
[physical hardware support matrix](docs/hardware-support.md).

## Microkernel core

The Ring 0 crate contains only boot, memory, interrupt, IPC, and scheduling
mechanisms. Drivers, filesystems, networking, policy, and identity belong in
isolated user address spaces.

- `task` defines fixed-size thread contexts and distinct kernel/user execution
  modes without a kernel heap.
- `scheduler` provides 64 generation-checked thread slots. Cooperative threads
  run until they yield or block. Real-time threads use fixed priority, then
  earliest deadline, and can preempt lower-ranked work on a timer tick.
- `ipc` is a bounded, non-blocking MPMC queue. Messages contain small control
  words plus shared-region descriptors, so payload bytes stay in mapped pages
  instead of being copied through the kernel.
- `capability` keeps 256 generation-checked object tokens and their derivation
  links in fixed kernel memory. Address spaces must present fine-grained rights
  for thread creation, memory mapping, and IPC. Delegation can only attenuate
  rights, and a parent token can revoke its complete delegation subtree.

This core has no driver, filesystem, network stack, dynamic allocator, or POSIX
compatibility layer. Those are user-space services communicating over IPC.

## Memory-safe system model

Safe kernel-independent crates forbid `unsafe` code. Hardware and firmware
crates deny implicit unsafe operations, keeping raw register, MMIO, page-table,
firmware, and ABI access inside explicit boundary blocks. The lock-free kernel
IPC queue stores every message field in atomics and needs no unsafe cell.

`synos-system-model` replaces mutable global directory conventions with sealed
root manifests. A root binds logical service names to SHA-256 package IDs, and
activation validates the complete manifest before replacing the active root.
Packages and dependencies resolve only by immutable digest. Each process gets a
bounded package namespace, so undeclared host paths and packages are invisible.
Package payloads live as versioned SynFS objects; the system model is their
heap-free metadata and activation layer. `SynFsRepository` writes payload and
manifest objects under digest-derived names, verifies existing objects before
reuse, and records every activated root as a new `system/root.manifest`
version.

## Async platform I/O and media

`synos-platform-io` provides generation-checked, fixed-capacity asynchronous
request/completion queues. Its API has submit, dispatch, complete, cancel, and
poll operations, with no blocking compatibility call.

Storage and device requests carry capability-mapped shared-buffer descriptors.
The same descriptor contract drives unified audio and video packets for
present, capture, encode, and decode operations. Multi-plane media payloads
therefore stay in shared pages instead of being copied through IPC messages.

## Legacy x86 PC drivers

The `synos-legacy-pc-drivers` crate provides heap-free Ring 3 driver building
blocks. It enumerates conventional PCI configuration space, identifies AHCI and
NVMe controllers, prepares their DMA queues and commands, and exposes descriptor
ring hooks for Intel E1000-family and Realtek RTL8169-family Ethernet devices.
Port and MMIO access remain capability-controlled by the platform service.

## User-space async networking

`synos-netd` is a heap-free Ring 3 TCP/IP service built on `smoltcp`. NIC
drivers loan fixed packet slots to the stack, so ingress and egress frames stay
in their original buffers while queue ownership changes. Polling accepts an
ingress budget to keep network work bounded under load.

Applications submit socket operations through shared IPC rings. Each channel
has an authenticated principal and a maximum rights mask; every socket token is
generation-checked, owner-bound, and separately authorizes listen, connect,
send, receive, inspect, and close operations. Payload descriptors are validated
against capability-mapped shared regions before the daemon touches them.

## Web applications and microservices

`synos-http` provides heap-free HTTP/1 request parsing, response encoding, and a
fixed-capacity method/path router for Ring 3 services. Routes carry web-service
rights, so the authenticated principal and attenuated grant are checked before
handler dispatch. Its gRPC layer validates `application/grpc` requests, applies
the same route rights, and frames bounded protobuf messages without reflection
or allocation.

The asynchronous HTTP server owns no raw network backend. It submits bounded
open, listen, receive, send, and close operations through `synos-netd` IPC
rings, and every operation carries the generation-checked socket capability
returned by that daemon. Callers drive both services cooperatively and provide
capability-mapped request and response buffers.

## Service isolation and fault recovery

`synos-init` supervises fixed-capacity Ring 3 storage, network, and system
services. A panic or protection fault fences only the dead process' capabilities,
IPC endpoints, DMA mappings, and interrupts. The service then restarts with a
new generation after bounded exponential backoff; unrelated services remain
running.

Cluster failure recovery uses two-phase DLM fencing. Heartbeat failure first
advances the node's membership epoch, making old lease requests and renewals
stale. After the transport confirms NIC and CXL isolation, the DLM may evict
the node's locks and the DSM layer may release its memory leases and recover
page ownership. DSM recovery rejects nodes that have not reached the isolated
state.

On legacy BIOS machines, stage 2 selects a 32-bit linear VESA mode when one is
available. On UEFI machines, the loader passes the current GOP framebuffer.
The early kernel console writes there and falls back to VGA text mode when no
linear framebuffer was supplied. COM1 remains active in every case.

## SynFS day-one core

`synos-synfs` is a `no_std`, fixed-capacity filesystem core for the Ring 3
SynFS service. Metadata lives in immutable Copy-on-Write B+tree blocks. A write
creates the next file version, so `notes.txt`, `notes.txt;0`, and the highest
numbered version resolve to the latest contents while `notes.txt;1` selects an
exact immutable version.

File data is split into content-matched blocks. Unchanged tails are shared
between versions, and a failed or superseded tree update cannot damage the
committed root. `SynfsPurged` applies bounded per-file retention work, then
mark-and-sweep collection reclaims tombstoned data and abandoned CoW branches.

`StoragePoolAdmin` combines NVMe namespaces, CXL persistent-memory regions, and
network block targets into striped or fault-domain-separated mirrored pools. It
supports device registration, pool creation and growth, draining/failure health
changes, allocation accounting, safe detach, and degraded/offline reporting.

SynFS checkpoints pin immutable tree roots across later writes and garbage
collection. The `synos-backup` worker streams every live file version from one
checkpoint into a checksummed `SYNBACK1` archive through bounded cooperative
polls, then releases the root after the caller commits the completed backup.

## OpenVMS feature core

The kernel DLM implements the six OpenVMS lock modes for shared-memory, file,
and named resources. Lock requests require a matching resource capability,
conflicts queue in FIFO order, and all locks from a failed cluster node can be
released together.

`synos-system-model` provides process, job, group, system, and cluster logical-name
scopes. Entries target files, devices, or IPC channels and use owner-controlled
ACLs. Its command dictionary accepts positional arguments and DCL-style
qualifiers, validates Boolean, integer, and text values before dispatch, and
returns bounded structured fields.

SynFS stores sequential and indexed RMS record files as ordinary immutable file
versions. Callers supply serialization scratch space and can select records by
position or indexed key; flat byte-stream files continue to use `write` and
`read`.

`synos-rms` adds the application-facing record API. `RecordFile` supports
create, read, insert, update, and delete operations, while `StructuredRecord`
lets applications bind their own bounded binary codecs. Record selectors can
be resolved to stable ordinal ranges and protected through `DlmRecordLocks`;
the native DLM binding maps read/update access to protected-read/protected-write
locks and uses one-byte ranges to avoid unrelated-record contention.

The same crate provides a fixed-capacity embedded key-value database. Binary
keys are encoded into the SynFS B+tree namespace, values remain normal
versioned CoW files, and mapped snapshots expose immutable value pages without
copying. Bounded transactions stage operations without a heap and publish one
new SynFS root only after every put and delete succeeds. Dropped or failed
transactions restore the old root and reclaim abandoned blocks.

`synos-status` defines the common 32-bit condition layout used by kernel,
filesystem, driver, command, and system-model errors. It keeps the OpenVMS
odd-value success convention while exposing facility, code, severity, and flag
fields without platform-sized error values.

`syn-script` builds native command procedures on those primitives. It compiles
bounded DCL-style statements, validates every pipeline stage against the typed
command registry, and passes structured output records between stages through
checksummed shared-memory IPC payloads. Scripts can define scoped logical
names, create typed symbols, and attenuate cryptographic capability tokens;
logical-name changes still pass through both namespace capabilities and entry
ACLs. `$STATUS`, `IF SUCCESS`, `IF FAILURE`, `SET ON`, `SET NOON`, and
`ON ERROR THEN` provide deterministic condition handling without host shell
exit-code conventions.

For agent-generated procedures, `syn-script` reflects the live typed command
registry into deterministic function-tool JSON schemas. Function names map
back to command routes without stringly typed dispatch. `SandboxExecutor`
drives each typed pipeline command exactly once through a handler bound to an
exclusive private SynFS root: the agent can read and validate staged state,
then either discard it or atomically publish the whole transaction. Dropped and
failed sandboxes always restore the original generation, and every finish
returns a base/staged/result generation receipt.

`synos-agent-bridge` joins those primitives into the native AI execution
boundary. It exports live command schemas, derives short-lived capabilities
sealed to one agent and exact task rights, and consumes each grant once through
a fixed replay ledger. `RUN /SANDBOX` always discards its private SynFS root;
an approved normal run publishes that root only after successful script
completion and a write-authorized token. A prepared run can be inspected before
approval, so commit publishes the exact validated root without rerunning agent
commands.

## Embedded scripting and Wasm extensions

`synos-embedded-script` embeds Rhai for service automation. Each evaluation has
hard limits for source size, instructions, recursion, expression depth,
functions, variables, collections, strings, and queued work. Scripts can only
enqueue operations named in their capability list; the owning service validates
and performs those requests after evaluation.

`synos-wasm-script` runs untrusted binary Wasm through the pure-Rust Wasmi
interpreter. It exposes no WASI filesystem, network, environment, or clock.
Each invocation gets strict compile limits, a module-size and fuel budget,
bounded memory/table resources, and a small numeric host ABI guarded by
per-operation capability masks. Missing imports, denied operations, resource
growth, bad entry signatures, and fuel exhaustion fail without widening guest
authority.

## Hardware fabric and clustering

`synos-fabric` gives CXL and legacy Ethernet clusters one bounded, heap-free
memory control plane. Its CXL 3.0/3.1 path accepts endpoints discovered through
PCI CXL DVSECs, validates Type-3 devices, and programs HDM decoder component
registers through an isolated MMIO boundary. Generation-checked leases allocate
aligned RAM or VRAM ranges and expire or release automatically when a node
fails.

The global address map resolves local RAM, CXL memory, remote layer-2 memory,
and VRAM through the same 64-bit address contract. A bounded sampler recommends
hot-page migration when observed latency justifies it; committing a verified
copy redirects later resolutions without changing its global address.

On legacy x86, the kernel can install a `#PF` resolver. The DSM protocol
fragments 4 KiB pages into raw Ethernet payloads under EtherType `0x88b5`,
rejects stale or malformed fragments, and grants read or exclusive mappings
only under live software DLM leases. Writers invalidate all remote sharers
before gaining a writable mapping.

Cluster heartbeats can run below one millisecond. A failure decision marks the
node unavailable, revokes its memory leases and coherence ownership, and makes
every mirrored pool resolve through its surviving node.

## Power and hardware lifecycle

The UEFI loader passes the ACPI RSDP into the kernel. `synos-power` validates
RSDP, RSDT/XSDT, FADT, and DSDT checksums without allocation, reads fixed-event
and reset registers, extracts `_S5` shutdown values and static thermal trip
points, and applies hysteresis-based throttling or emergency shutdown policy.
The kernel enables ACPI mode, handles the physical power button, and exposes
ACPI-backed `SHUTDOWN` and `REBOOT` commands with the legacy reset fallback.

CXL Type-3 devices and NVMe namespaces have explicit online, draining, and
removed lifecycle states. CXL pool draining blocks new memory leases and
refuses removal while leases remain. NVMe draining blocks new SynFS
allocations and refuses removal while a storage pool still claims the device.
HDM decoders and NVMe controllers expose quiesce operations for the final
hardware detach.

## Architectural risk mitigations

Fabric resolution now preserves transport semantics: local RAM and CXL HDM
windows are direct cache-coherent mappings, while layer-2 pools issue 4 KiB
remote-page requests into a tiered NUMA cache. The unified allocator detects
sequential read streams and feeds future non-local pages to a bounded
asynchronous prefetch queue.

Applications capability-map IPC rings once, then exchange records with daemons
using lock-free atomic operations. SynFS can bind a read capability to an
immutable CoW tree generation and expose its data pages directly; the
`MappedRecordFile` runtime validates and searches RMS records in-process without
per-record IPC or copying.

DLM leases may cover a whole object or an exact byte range. Disjoint ranges
make progress independently, overlapping waiters remain ordered, and epochs
prevent stale lease renewal. `EpochRcu` publishes read-heavy cluster snapshots
without reader locks and delays reclamation until registered readers pass a
quiescent state.

The bootstrap capability space remains fixed-size and heap-free. Physical
memory enters it as untyped tokens, user-space managers retype non-overlapping
ranges, and parent/child/sibling CDT links live beside each protected resource
descriptor. Authorized process and system logical names are published to
epoch-protected atomic hash pages for Ring 3 lookup.

## Authentication, authorization, and identity

`synos-auth` is the heap-free Ring 3 identity core. Its fixed authorization
database holds local and node-local user records with passkey, TPM 2.0, and SSH
public credentials. Authentication uses one-shot challenges and a platform
crypto verifier. A successful session builds the kernel-owned execution
persona and mints only the configured initial capability set into the login
address space.

Remote administration uses a stricter WebAuthn path. The daemon binds each
one-shot assertion to a server-supplied 256-bit ceremony nonce, frontend device,
relying-party ID, HTTPS origin, credential, and expiry. It requires both
authenticator user-presence and user-verification flags, so a platform passkey
must complete its device biometric or PIN check. COSE signature and
`clientDataJSON` validation stay behind a platform crypto boundary, while
`synos-auth` enforces ceremony type, origin and RP hashes, bounded inputs, and
monotonic authenticator counters.

The remote security gateway issues capabilities only after that passkey
session. Trusted code installs an exact resource allowlist; the frontend may
request only a subset of its safe rights and a lifetime of at most five
minutes. Issued tokens are bound to the authenticated device, restricted to
Layer 2, capped by the passkey session expiry, sealed against subject rebinding,
and fenced by a gateway revocation epoch. Remote tokens can never carry map,
create, delegate, or revoke rights.

Kernel personas carry dynamic rights identifiers such as `LLM_OPERATOR`,
`NETWORK_INBOUND`, and `BATCH_JOB`. A process may suspend, restore, or
permanently drop its own rights. Logical-name access now includes JOB scope and
passes through both its OpenVMS-style ACL and a scoped kernel capability.
Zero-copy IPC can delegate an object handle while attenuating its rights, and
capability revocation hooks notify memory and fabric owners before descendants
are removed from the derivation tree.

Cross-node authority uses wire-encoded HMAC-SHA256 capability tokens. Tokens
bind issuer, borrower, resource, transport, expiry, and revocation epoch;
Macaroon-style caveats can only narrow them. Remote DSM faults require both a
valid memory token and the matching live DLM epoch. Resource owners can lend
bounded RAM, VRAM, or compute units, then revoke the loan and request immediate
remote unmapping, DSM invalidation, or compute stop.

## Native applications and actor runtimes

`synos-app` defines the bounded `App.toml` application contract. A manifest
names an immutable image, application kind, node placement, restart policy, and
up to 16 exact capability requests. Before registration, the Ring 3 application
supervisor intersects those requests with an administrator policy. Missing
required resources, wrong object kinds, and rights escalation stop the spawn;
optional unavailable resources are omitted. Each restart fences the old process
and receives a fresh generation and only the approved capability set.

`synos-actors` gives local and distributed processes one actor API. Local
mailboxes use native IPC channels. Remote actor references use page-aligned
Software DSM mailboxes carrying a live write authority and DLM lease epoch.
Message routing, remote supervisor selection, process spawning, replies, and
stops are handled by the actor system, so actor code does not construct network
packets or choose a transport.

## Multi-cluster federation and sandboxing

Clusters discover peers through signed, expiring gateway announcements and
reject replayed epochs and nonces. Signed offers trade bounded CPU, RAM, and
VRAM leases over the transports advertised by that peer. A lending cluster can
send an authenticated revocation with a deadline below one millisecond.

Borrowed workloads run in kernel `BlindMicroSilo` scopes. Host process trees,
SynFS mounts, network sockets, and federation controls are always invisible.
CXL-IDE and SEV, TDX, or CCA memory encryption are required when the platform
advertises them; otherwise kernel page isolation remains active. Federation
epochs fence stale DLM locks after expiry, revocation, or cluster failover.

## LLM memory runtime

`synos-llm` presents many local and CXL fabric leases as one contiguous virtual
model allocation. Allocations use 64-bit sizes, may span multiple-terabyte
extents, and resolve page faults without exposing tensor or pipeline placement
to the model framework.

KV caches grow in stable token-addressed segments. The allocator fills local
RAM first, then CXL, then layer-2 memory, while requiring a mirror for every KV
segment. Existing token addresses do not move when a long-context request
needs another segment.

Each inference request has a checksummed recovery record on two journal nodes.
A token checkpoint becomes committed only after both copies acknowledge it.
When one node drops, a memory degradation handle resumes from the last committed
token, resolves mirrored model and KV memory through the fabric, and can seed a
replacement journal replica without changing the request identity.

## High-level AI execution

`synos-inference` is the bounded, `no_std` Ring 3 service layer above
`synos-llm`. Its transport-neutral gateway accepts OpenAI-compatible model,
text-completion, and chat-completion requests, emits JSON or server-sent
completion chunks, and also accepts length-prefixed gRPC/protobuf records.
`synos-netd` can carry it now; the native HTTP stack can bind it without
changing model execution.

The cluster inference service registers models already mapped in pooled memory.
Each request reserves a mirrored, growable KV cache in RAM or VRAM, starts a
dual-journal recovery ledger, and exposes explicit acknowledge, token
checkpoint, node-failure, replica-repair, and completion transitions.

Long-running agents use `AgentSnapshotter` to write checksummed execution
images as immutable SynFS file versions on a fixed interval. The newest image
pins its complete CoW filesystem generation for crash-consistent stack
recovery, while earlier state versions remain addressable for rollback.

## Pure-Rust AI compute

`synos-compute` gives Candle and Burn a small native SynOS runtime contract
without a C, C++, CUDA, or POSIX dependency. Fixed-rank tensor metadata points
directly into capability-mapped IPC regions; read-only and writable views
borrow those pages in place and validate shape, stride, bounds, alignment, and
access before a framework or device sees them.

GPU and NPU work stays in isolated Ring 3 drivers. PCI display and processing
accelerators become bounded device descriptors with capability-controlled BAR,
interrupt, and doorbell operations. Frameworks submit SPIR-V/Vulkan or native
compute kernels through generation-checked asynchronous queues, and tensor
bindings continue to name the original shared pages instead of staging copies.

## Target platforms and emulation

The platform kit under `platforms` defines evidence-gated profiles for two
consumer PCs, a local QEMU/KVM cluster, switched CXL racks, and PCIe or NVLink
GPU fabrics. A target counts as qualified only when its captured boot,
inventory, fabric, migration, and failover evidence passes
`scripts/qualify-platform.sh`.

On Linux, `scripts/qemu-cluster.sh` launches two or more q35 guests with unique
E1000 NICs on one multicast Ethernet bus. Every guest receives a QEMU CXL
Type-3 endpoint and an `ivshmem-plain` region shared by all guests. The
companion failure-injection command terminates one selected node so heartbeat,
lease cleanup, mirrored redirection, and inference recovery can be observed.

## Docker-based local testing

A multi-stage Docker image builds SynOS from source and bundles QEMU so you can
run a bootable SynOS guest on any Docker host without installing Rust, clang,
or QEMU locally. The guest exposes a VNC display; connect with any VNC client
to see the SynOS console.

### Quick start — single node

```sh
# Build the Docker image (takes several minutes on first run)
docker build -t synos:latest .

# Launch a single SynOS guest with serial on stdout + VNC on :0
docker run --rm -it -p 5900:5900 synos single
```

Open a VNC client to `localhost:5900` (password is empty). Press Ctrl-C to stop
the guest.

### Docker Compose

```sh
# Single node (foreground, Ctrl-C to stop)
docker compose up synos

# Three-node CXL cluster
docker compose --profile cluster up synos-cluster
```

### Cluster mode

```sh
# 2–8 node cluster with CXL fabric and shared ivshmem region
docker run --rm -it \
  -p 5900-5907:5900-5907 \
  -e SYNOS_CLUSTER_NODES=3 \
  synos cluster
```

Each node receives its own VNC display starting at port 5900. Serial logs for
every node are written to `/tmp/synos-qemu/node-*.serial.log` inside the
container.

### Environment variables

| Variable               | Default                              | Description                              |
| ---------------------- | ------------------------------------ | ---------------------------------------- |
| `SYNOS_DISK_IMAGE`     | `/synos/build/bios/synos-bios.img`   | Path to the raw disk image               |
| `SYNOS_CLUSTER_NODES`  | `1`                                  | Number of cluster guests (2–8)           |
| `SYNOS_GUEST_MEMORY`   | `512M`                               | RAM per guest                            |
| `SYNOS_VNC_BASE`       | `5900`                               | Starting VNC port                        |
| `SYNOS_QEMU_ACCEL`     | `tcg`                                | QEMU accelerator: `tcg` or `kvm`         |
| `SYNOS_QEMU_EXTRA`     | (empty)                              | Extra flags appended to QEMU             |

### Platform notes

- On Linux with `/dev/kvm` accessible, set `SYNOS_QEMU_ACCEL=kvm` for
  near-native speed.
- On macOS and Windows, TCG software emulation is used. Performance is adequate
  for interactive shell testing.
- The Docker image includes both the BIOS raw image and the portable UEFI image;
  set `SYNOS_DISK_IMAGE` to `/synos/build/portable/synos.img` to boot via UEFI.

## Native interactive shell

`syn-shell` is a heap-free Ring 3 shell core. Its UTF-8 line editor provides
cursor editing and bounded history. The parser supports quoted values,
DCL-style qualifiers and negated Boolean qualifiers, comments, structured
pipelines, and trailing `&` background submission.

Commands are registered with typed positional and qualifier specifications.
The complete invocation is validated before an executor receives it. Pipeline
stages exchange `StructuredOutput` objects instead of text, while the terminal
renderer can emit list or JSON views.

Built-in diagnostics include:

```text
SHOW MEMORY
SHOW MEMORY/CLUSTER
SHOW PROCESS 42
SHOW OBSOLETE/CLUSTER
MONITOR /INTERVAL=250000 /SAMPLES=20
```

Cluster administration uses the same typed DCL surface. The cluster command
registry keeps stable routes and emits structured records for pipelines and
JSON rendering:

```text
SHOW CLUSTER/MEMBERS
SHOW CLUSTER
SHOW CLUSTER/TOPOLOGY
SHOW CLUSTER/HEALTH
SHOW CLUSTER/RESOURCES
SHOW CLUSTER/CONFIG
LIST CLUSTERS /FEDERATED /LIMIT=20 /PAGE=2
CREATE CLUSTER compute /DESCRIPTION="local fabric" /QUORUM=3
JOIN CLUSTER /INVITATION="token" /ENDPOINT="10.0.0.2"
REMOVE NODE node-7 /FORCE /CONFIRM
HELP SHOW CLUSTER
```

Two-word forms and canonical hyphenated forms are equivalent. Destructive
cluster actions require `/CONFIRM`; `/FORCE` adds the stronger safety check.
`SHOW CLUSTER` returns stable fields for identity, lifecycle status, leader,
coordinator, membership counts, quorum, health, aggregate capacity, and
control/data protocol versions. The `/MEMBERS` and `/TOPOLOGY` views expose
bounded numbered records with continuation fields; `/HEALTH`, `/RESOURCES`,
and `/CONFIG` expose typed aggregate snapshots.

Cluster permissions are capability-based. Administrators may create, modify,
invite, fence, recover, and retire clusters. Operators may join, leave, remove,
modify, invite, fence, and manage resources. Auditors and read-only users may
inspect state only. Node owners may join, leave, and manage their own resources.

`/CONFIRM` is required for leave, remove, federation removal, fencing, recovery,
rollback, abandonment, and node admission decisions. `/FORCE` is an extra
authorization check; it never replaces `/CONFIRM`. Mutable configuration
commands support `/DRY_RUN`.

Common status results are `normal`, `invalid argument`, `access denied`,
`confirmation required`, `quorum lost`, `cluster partitioned`, `protocol
mismatch`, `stale state`, `node unsafe`, `reconciliation required`, and
`recovery state invalid`. A failed quorum makes the cluster read-only. A
partition or stale membership epoch blocks writes until an administrator
reconciles or fences the unsafe node.

Safe recovery order is: inspect `SHOW CLUSTER/HEALTH`, stop new work with
`DRAIN NODE`, fence unsafe nodes, reconcile membership/SynFS/leases, then use
`RECOVER NODE` or `REJOIN NODE`. Use `ABANDON NODE /FORCE /CONFIRM` only when
data reconciliation is impossible. `REMOVE CLUSTER` is destructive and must
follow workload, lease, membership, and storage checks.

The opt-in two-node QEMU validation captures command input, serial output, QMP
control traffic, failure injection, and node logs:

```sh
SYNOS_RUN_QEMU_TESTS=1 ./scripts/qemu-cluster-validation.sh
```

The runner covers create, list, show, join/leave, federation, fencing,
recovery, rejoin, and a failed-node path. It needs Linux QEMU with CXL,
ivshmem, and multicast socket support.

EDIT file (also EDT) opens a bounded UTF-8 full-screen editor. It edits a
selected version and saves as a new SynFS version; an omitted selector opens
the latest version. Ctrl-S saves, Ctrl-Z saves and exits, and Ctrl-X discards
and exits. Escape enters command mode: I inserts, S saves, E saves and exits,
Q quits, while Y, X, and P copy, cut, and paste the current selection.
Shift plus arrows or Home/End selects text. Unsaved changes prompt before
discard. If another save reaches the file first, the editor reports a conflict
and requires an explicit confirmation before publishing another version.

## Cluster topology monitor

`synos-top` is a heap-free real-time dashboard core. It renders per-node RAM
and VRAM heatmaps, p50/p99 remote DSM page-fault latency, and the live
capability derivation tree. Its fixed-rate sampler keeps the last coherent
snapshot when a telemetry read fails.

The renderer emits one ANSI stream for serial terminals and the kernel's VGA
or GOP framebuffer console. A logarithmic per-node latency tracker calculates
interval percentiles without allocation. Trusted diagnostics can enumerate
live kernel capability descriptors through `CapabilitySpace::entries`; the
serving process remains responsible for filtering what a caller may see.

`synos-mesh` carries signed, expiring cluster advertisements. Each advertisement
contains the cluster and node identity, protocol versions, capabilities, and up
to four ordered endpoints. Endpoints support CXL, Ethernet, wireless, 5G,
loopback, and tunnels, including direct, NAT, relay, and offline routes.

`TopologyGraph` keeps bounded node and link records with zones, reachability,
latency, bandwidth, transport, route, and MTU. `ConnectivityManager` rotates
failed endpoints, negotiates the smallest MTU, applies capped exponential
backoff, and retains advertisements for offline inspection. The shell exposes
these records through `SHOW CLUSTER/TOPOLOGY`; the Rust and Swift clients use
the same versioned topology RPC.

The asynchronous interpreter never waits inside command dispatch. Background
commands and complete pipelines enter a system-wide bounded job queue with
priorities, start times, dependencies, retry limits, worker leases,
cancellation, and lost-worker recovery.

## Cross-platform client SDKs

`synos-client-sdk` is a `no_std`, allocation-free client and frontend gateway
contract for macOS, iOS, Android, and WebAssembly. Its versioned `SYRP` frames
carry optional 192-byte cryptographic capabilities and provide typed RPCs for
cluster snapshots, job submission, and capability delegation. Platform code
implements one `RpcTransport` round trip, so native HTTP stacks and browser
`fetch` can share the same protocol without pulling sockets or an executor into
the SDK.

`FrontendGateway` decodes and bounds-checks requests before calling a
policy-owning `GatewayService`. Malformed input never reaches service handlers,
and remote failures return stable protocol status codes.

Cluster lifecycle, membership, invitations, plans, health, resources, topology,
and audit activity share bounded SDK schemas. Subscriptions use cursors and a
fixed maximum poll batch, so a slow dashboard cannot grow server state without
bound. `synos-observability` exports dimensioned metric samples, trace/log
records, audit records, and alerts keyed by cluster, node, transport, workload,
and operation.

`clients/apple` contains a shared Swift implementation and SwiftUI control
surface for macOS 14+ and iOS 17+. It displays node health and resource use,
submits bounded jobs, and requests restricted capability grants through a TLS
HTTP gateway.

## Remote console and display

`synos-webterm` provides a heap-free VT100/VT420/DECterm terminal model for
browser and native clients. It handles UTF-8, cursor and scrolling regions,
erase and insertion operations, SGR colors and attributes, DEC private modes,
and OSC framing. Dirty rows become fixed WebGPU cell instances, allowing a
WebAssembly frontend to update only changed GPU buffer ranges.

Its Ring 3 SSH gate accepts public-key proofs through a platform authentication
trait, requires an authenticated shell capability before opening `syn-shell`,
and binds every generation-checked session to its principal. Terminal resize,
bounded input, output polling, and close operations stay transport-independent.

`synos-remote-display` moves capture surfaces through explicit available,
capturing, encoding, and in-flight lease states. NV12, P010, RGBA, and BGRA
planes remain in capability-mapped shared memory. The AV1 encoder boundary
returns another shared descriptor, and the RFC 9364 RTP packetizer creates
scatter/gather views into that bitstream instead of copying payload bytes.

The display daemon negotiates AV1-only WebRTC media sessions, validates
expiring view/input grants, tracks RTP sequence and SSRC state, and applies
loss, latency, and acknowledged-bitrate feedback. Keyframe requests and
adaptive encoder settings keep the path suitable for low-latency phones,
tablets, and desktop browsers.

## Rust toolchain and runtime

Ring 3 Rust programs target `targets/x86_64-unknown-synos.json` or
`targets/aarch64-unknown-synos.json`. Both targets produce position-independent
static images with abort-on-panic behavior.

`synos-runtime` provides the native `sys::synos` platform contract used by the
SynOS `std` port. The loader installs one call gate and an initial capability
set. Files, threads, wait words, clocks, memory mappings, IPC endpoints, and
SynFS objects therefore use generation-checked handles instead of an ambient
Unix syscall namespace.

`synos-ipc` owns the shared MPMC ring used by Ring 0 and Ring 3. Fixed envelopes
move through atomics while structured payloads stay in capability-mapped pages.
`zerocopy` validates typed views over those pages, and `SharedArena` archives
messages without heap allocation.

Legacy C and C++ components can opt into `synos-posix-compat`. It maintains a
bounded process-local file-descriptor table and translates open, close, read,
write, seek, and clock operations into SynFS capabilities and shared-buffer
descriptors. Its C contract is in
`crates/posix-compat/include/synos_posix.h`.

The same Ring 3 compatibility layer decodes x86_64 and AArch64 Linux syscall
vectors for read, write, open/openat, close, lseek, monotonic clock, getpid,
and exit. The trap adapter supplies a capability-checked `LinuxUserMemory`
view, allowing shared pages to stay mapped instead of using a syscall bounce
buffer. `/proc`, `/sys`, and `/dev` paths resolve through the process logical
name fast path (`PROC_*`, `SYS_*`, and `DEV_*`) and retain the target's file,
device, or IPC kind.

Container address spaces can use `ZeroCopyContainerMemory` with the existing
fabric allocator. One virtual range is backed by generation-checked leases
over local RAM, CXL memory, and optional layer-2 Software DSM extents. Page
resolution follows the active fabric mapping, so migration and failover do not
copy the container's byte stream.

## Rust package toolchain

`cargo-synos` builds Ring 3 programs for either SynOS target, builds the required
`core` and `alloc` libraries from the pinned `rust-src`, and creates signed
binary bundles:

```sh
cargo install --path tools/cargo-synos
cargo synos package \
  --bin example-service \
  --target x86_64 \
  --release \
  --key package-signing.key \
  --output example-service.synpkg
```

The key file contains either 32 raw bytes or 64 hexadecimal characters.
`--dependency` pins another package by its 64-character SHA-256 ID. The
standalone `build` and `bundle` commands expose each half of the workflow.

Compile a native Ring 3 binary with the reusable compiler driver:

```sh
cargo run -p cargo-synos -- synos compile \
  --manifest-path examples/hello-world/Cargo.toml \
  --bin hello-world --target x86_64
```

Run the same example on the host:

```sh
cargo run -p cargo-synos -- synos run \
  --manifest-path examples/hello-world/Cargo.toml \
  --bin hello-world
```

The compiler emits a position-independent SynOS binary. Package it with the
existing signing command before installing it into SynFS.

Compile every production Ring 0 and Ring 3 library crate used by the TODO roadmap:

```sh
cargo run -p cargo-synos -- synos compile-all --target x86_64 --release
```

The workspace command excludes only host-side Cargo tooling, test fixtures,
the UEFI host application, and the host virtual machine. See
[`docs/native-compiler.md`](docs/native-compiler.md) for the boundary.

The heap-free `synos-pkg` daemon rejects bundles from unknown trust keys,
validates the signature and payload digest, and installs payloads and manifests
under SHA-256-derived SynFS names. A bounded `SystemConfiguration` declares the
complete logical-name-to-package mapping. Activation first validates every
package and dependency, then commits one new `system/root.manifest` version, so
readers see either the old root or the complete new root.

## Boot from USB

This bootstrap is experimental. Use a spare USB drive. The commands below erase
the selected drive completely. Check the drive name and size twice before
running them.

There are two different boot methods. The current build does not make one
hybrid USB that supports both.



### UEFI

Use this method on an x86_64 UEFI computer. Secure Boot must be disabled because
the loader is not signed.

Build the UEFI application:

```sh
./scripts/build-uefi-loader.sh
```

Format the USB drive as GPT with a FAT32 partition. On macOS:

```sh
diskutil list
diskutil eraseDisk FAT32 SYNOS GPT /dev/disk5
mkdir -p /Volumes/SYNOS/EFI/BOOT

cp target/x86_64-unknown-uefi/release/synos-loader.efi \
    /Volumes/SYNOS/EFI/BOOT/BOOTX64.EFI
sync
diskutil eject /dev/disk5
```

On Linux, after creating and mounting a FAT32 EFI System Partition:

```sh
mkdir -p /Volumes/SYNOS/EFI/BOOT
cp target/x86_64-unknown-uefi/release/synos-loader.efi \
  /Volumes/SYNOS/EFI/BOOT/BOOTX64.EFI
sync
diskutil eject /dev/disk5
```

Only one file is copied. It must have this exact path and name:

```text
EFI/BOOT/BOOTX64.EFI
```

Choose the UEFI USB entry in the firmware boot menu. Current UEFI screen output
shows a `SynOS bare-metal bootstrap` prompt. Press a key to start the kernel.
If firmware handoff fails, the loader displays the EFI status instead of
silently returning to another operating system.

After rebuilding, always replace `EFI/BOOT/BOOTX64.EFI` on the USB drive with
the new `synos-loader.efi`.


### Legacy BIOS

Use this method on an x86_64 computer with Legacy Boot or CSM enabled. Secure
Boot must be disabled.

Build the image:

```sh
./scripts/build-bios-image.sh
```

Do not copy individual files to the USB drive. Write the complete
`build/bios/synos-bios.img` image to the whole drive, not to a partition.

On macOS:

```sh
diskutil list
diskutil unmountDisk /dev/diskN
sudo dd if=build/bios/synos-bios.img of=/dev/rdiskN bs=4m
sync
diskutil eject /dev/diskN
```

Replace `diskN` with the USB drive.

On Linux:

```sh
lsblk -p
sudo umount /dev/sdX1
sudo dd if=build/bios/synos-bios.img of=/dev/sdX bs=4M status=progress conv=fsync
sudo eject /dev/sdX
```

Replace `sdX` with the whole USB drive. Unmount every mounted partition if the
drive has more than one.

Boot the computer's one-time boot menu and choose the USB drive under its
Legacy or CSM entry. A successful start prints `SynOS kernel bootstrap` on VGA
and COM1 serial.


## Dual boot without repartitioning

The UEFI loader presents a boot menu for SynOS, Windows Boot Manager, and GRUB.
It searches every firmware-visible EFI System Partition for the standard
Windows path and common GRUB paths. A selected child loader runs through UEFI
`LoadImage`/`StartImage`; SynOS does not alter firmware boot variables.

To keep SynOS as one ordinary file on an existing EXT4 or NTFS partition, copy
`build/portable/synos.img` to the partition root and add the contents of
`boot/grub/grub.cfg` to the host GRUB configuration. GRUB finds the file,
mounts it as a loopback FAT image, and starts its fallback UEFI loader. No
partition-table change is needed.

## Read-only host filesystems

`synos-host-filesystems` is a heap-free Ring 3 storage building block. It scans
MBR and GPT partition tables, detects FAT32, EXT4, and NTFS volumes, resolves
paths, and provides bounded positional file reads. FAT long names, EXT4 extent
trees, NTFS MFT data runs, large directory indexes, resident data, sparse data,
and non-resident data are supported.

The public `ReadAt` device contract has no write operation. Host volumes
therefore stay read-only even if a caller has a storage-controller capability.
EXT4 journal replay and NTFS compressed, encrypted, or deduplicated files are
not supported; put model weights in ordinary uncompressed files.


### Serial console

COM1 uses 38400 baud, 8 data bits, no parity, and 1 stop bit. A USB-to-serial
adapter connected to the target machine can capture the earliest boot output.


### SynOS-shell commands

````

HELP
SHOW SYSTEM
REBOOT
SHUTDOWN

```
## Multi-tenant quotas

Every live kernel capability owns fixed-size resource state. Administrators
with `CONTROL` can configure independent token buckets for IPC messages, page
faults, and allocation bytes, plus an outstanding-memory ceiling. IPC sends and
receives use the `*_at` APIs with the scheduler clock; failed queue operations
refund their token. Pager and frame-allocation paths expose the same capability
charge, so tenants cannot bypass limits through another kernel entry point.

CXL userspace fabric managers configure `CxlBandwidthQos` per node/channel.
`GlobalAddressSpace::resolve_with_cxl_qos` admits each transfer by byte count
and returns a busy result when a tenant would consume another workload's
channel budget.
