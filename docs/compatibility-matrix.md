# SynOS compatibility matrix

The shared public-API registry and migration procedure are in
[`api-versioning.md`](api-versioning.md).

This table is the operator-facing map of the compatibility contracts currently
implemented by SynOS. The reader and decoder in source remain authoritative;
this page does not promise compatibility beyond the versions listed here.

## Compatibility rules

| Term | Meaning |
| --- | --- |
| Read/write | The current reader accepts the listed version and the current writer emits it. |
| Read/convert | The reader accepts the older version and has an explicit conversion path. |
| Negotiated | Peers select the highest version in the intersection of their advertised ranges. |
| Build-only | The target or schema can be built, but acceptance evidence is not a compatibility promise. |
| No downgrade | Make a new artifact with the older writer; do not edit bytes in place. |

## On-disk formats

| Artifact | Identifier and current version | Compatibility | Downgrade / recovery boundary | Authority |
| --- | --- | --- | --- | --- |
| SynFS volume | `SYNFSVOL`, volume `3`; `SYNFSMAP`; tree `SYNT`/`1` | Read/write v3 | Older readers reject v3; export/import into a target volume | [`synfs/src/volume.rs`](../crates/synfs/src/volume.rs), [`persistence-compatibility.md`](persistence-compatibility.md) |
| RMS record image | `SYNRMS01` | Read/write current fixed layout | No in-place downgrade; convert records | [`synfs/src/rms.rs`](../crates/synfs/src/rms.rs) |
| SynFS backup stream | `SYNBACK1`/`1`, trailer `SYNBEND1` | Write/export v1; no restore decoder | Preserve a complete pinned stream; it is not a mountable restore input | [`synos-backup/src/lib.rs`](../crates/synos-backup/src/lib.rs) |
| Legacy shell store | `SYNFS001`/`1` | Read/write legacy v1 | Invalid data falls back to the built-in store; no converter | [`persistence-compatibility.md`](persistence-compatibility.md) |
| SynOS system disk | `SYNOSDSK`/`1`, `SYNMANIF`, `SYNSET01` | Read/write v1 with two manifest slots | Unknown versions reject; recovery selects the newest complete checksum-valid slot | [`system_disk.rs`](../virtual_machine/src/devices/storage/system_disk.rs), [`persistence-compatibility.md`](persistence-compatibility.md) |
| Boot service manifest | `SYNSVC01`/`1` | Read/write v1; bounded role/path/checksum entries | Unknown versions, duplicate roles, bad paths, and package checksum failures reject | [`service_manifest.rs`](../crates/synfs/src/service_manifest.rs) |
| Guest persistence tail | `SYNOPS01`/`1` | Read/write v1 | Invalid metadata is treated as empty state; no in-place downgrade | [`persistence.rs`](../virtual_machine/src/devices/storage/persistence.rs) |
| Observability journal record | `SLOG`/`1`, exactly 128 bytes | Read/write v1 with checksum | Reject unknown versions, bad fields, and bad checksums | [`observability/src/lib.rs`](../crates/observability/src/lib.rs) |
| VM replay trace | `SYNREP01`/`1` | Read/write v1 | Version mismatch or trailing bytes is corruption; no downgrade | [`synos-replay/src/lib.rs`](../crates/synos-replay/src/lib.rs) |
| Cluster metadata | `SYNCLID1`/`1` | Read/write v1 | Checksum and generation must remain valid; no in-place downgrade | [`cluster.rs`](../crates/synos-storaged/src/cluster.rs) |
| Membership registry | `SYNMEMB1`/`1` | Read/write v1 | Rebuild stale state through quorum-approved recovery | [`membership.rs`](../crates/synos-storaged/src/membership.rs) |
| Cluster bootstrap | `SYNBOOT1`/`1` | Read/write v1 with signature and checksum | Revalidate trust and epoch; rotate through a new record | [`bootstrap.rs`](../crates/synos-storaged/src/bootstrap.rs) |
| Admission audit | `SYNADIT1`/`1` | Read/write v1 with bounded records | Corruption is an evidence failure, not an empty journal | [`admission.rs`](../crates/synos-storaged/src/admission.rs) |
| Mount catalog | `SYNMNT01`; monotonic catalog version | Read/write current layout | Layout changes need a new magic or explicit converter | [`state.rs`](../crates/synos-storaged/src/state.rs) |
| KVD state | `SYNKVD01` | Read/write current bounded layout | Invalid cache state is discarded; no downgrade | [`synos-kvd/src/lib.rs`](../crates/synos-kvd/src/lib.rs) |

The complete storage and recovery discussion is in
[`persistence-compatibility.md`](persistence-compatibility.md). External host
formats (Ext4, FAT32, and NTFS) are read-only inputs, not SynOS-owned formats.

## Wire protocols

| Boundary | Version | Compatibility contract | Authority |
| --- | ---: | --- | --- |
| Shared transport guard: HTTP, gRPC, SDK, remote terminal, mesh, cluster | `1` | Negotiated version ranges, replay window, authentication, size, and backpressure limits | [`protocol/src/lib.rs`](../crates/protocol/src/lib.rs), [`protocol-compatibility.md`](protocol-compatibility.md) |
| Client RPC frames (`SYRP`) | `1`; 24-byte header; 4096-byte maximum | Rust and Swift clients use the same frame version and method IDs | [`client-sdk/src/wire.rs`](../crates/client-sdk/src/wire.rs), [`SynOSClient.swift`](../clients/apple/Sources/SynOSClient/SynOSClient.swift) |
| Boot handoff (`BootInfo`) | magic `SYNOSBOO`, version `1` | Bootloader and kernel must agree on the exact version; unknown versions reject | [`boot-protocol/src/lib.rs`](../crates/boot-protocol/src/lib.rs) |
| Netd socket IPC | protocol `1`; request/response schemas `SYNNETRQ`/`SYNNETRS` | Exact version and schema required | [`netd/src/protocol.rs`](../crates/netd/src/protocol.rs) |
| VM migration checkpoint | protocol `3` | Authenticated, bounded `SYNOMIG3` stream; schema and feature negotiation required | [`control.rs`](../virtual_machine/src/control.rs), [`migration.rs`](../virtual_machine/src/migration.rs) |

Traffic limits, replay behavior, and reconnect policy are maintained in the
[protocol compatibility contract](protocol-compatibility.md).

## Snapshots

| Snapshot boundary | Current versions | Older-version behavior | Compatibility requirement |
| --- | --- | --- | --- |
| Raw VM snapshot payload | `SYNOVM01`, versions `1..2` | v1 reads and converts to v2 with `ALL` feature flags | Match guest RAM size; host-owned devices, disks, queues, and clocks are rebuilt or excluded |
| Authenticated snapshot envelope | `SYNOSIG1`, auth format `1`, HMAC-SHA256 | No unauthenticated restore | Persist or transmit only authenticated envelopes with the configured key |
| Snapshot schema negotiation | local range `1..2` | Highest common version is selected | Required feature bits must be present; unsupported bits reject |
| Snapshot chain/diff | Uses the snapshot payload version above | Explicit base snapshot required | Apply only to a compatible base; checksum and page bounds are validated |

Source: [`snapshot.rs`](../virtual_machine/src/snapshot.rs),
[`migration.rs`](../virtual_machine/src/migration.rs), and the detailed
[`persistence-compatibility.md`](persistence-compatibility.md) VM section.

## Package and configuration schemas

| Artifact | Current schema | Compatibility | Authority |
| --- | ---: | --- | --- |
| Signed package bundle | `SYNBNDL1`, version `1` | Read/write v1; signature, content IDs, dependencies, and lengths are mandatory | [`pkg/src/lib.rs`](../crates/pkg/src/lib.rs) |
| Application bundle | `SYNAPP01`, version `1` | Read/write v1; outer metadata and inner package must both validate | [`pkg/src/lib.rs`](../crates/pkg/src/lib.rs) |
| Provenance chain | `SYNPROV1` | Read/write current fixed schema | Content and signature links must remain complete | [`pkg/src/lib.rs`](../crates/pkg/src/lib.rs) |
| Toolchain archive | `SYNTOOL1`, version `1` | Read/write v1; signed asset paths and content IDs are checked | [`persistence-compatibility.md`](persistence-compatibility.md) |
| Declarative system configuration | schema `1` | Read/write v1; signed canonical source activates atomically | [`synos-declarative/src/parser.rs`](../crates/synos-declarative/src/parser.rs) |

Package downgrade means rebuilding and signing with the older toolchain. Never
change a bundle or activated configuration in place.

## Target triples

| Target triple | Architecture and image contract | Compatibility status |
| --- | --- | --- |
| `x86_64-unknown-synos` | 64-bit x86, PIC, static, abort-on-panic, `x86_64-unknown-none` LLVM target | Accepted target; primary build and boot evidence |
| `aarch64-unknown-synos` | AArch64 v8a, PIC, static, abort-on-panic, `aarch64-unknown-none` LLVM target | Build/cross-build target; not a full acceptance promise until runtime, linker, loader, and boot evidence exist |

The target JSON files and acceptance policy are documented in
[`targets/README.md`](../targets/README.md). Target compatibility requires the
pinned Rust channel and custom `core`/`alloc`/`std` build, not just a successful
Cargo compile.

## Client SDKs

| SDK | Package/tool version | Wire compatibility | Platform status |
| --- | --- | --- | --- |
| Rust `synos-client-sdk` | crate `0.1.0` | `SYRP` v1; shared transport guard v1 | `no_std`; transport supplied by the host; same contract for native and Wasm clients |
| Swift `SynOSClient` | Swift tools `6.0`; protocol constant `1` | `SYRP` v1; same 24-byte header, method IDs, and status values | iOS 17+ and macOS 14+ |

No SDK version is compatible merely because it compiles. A client must match
the wire version, method IDs, frame limits, status mapping, and negotiated
transport policy listed above.

## Updating this table

When a format or protocol changes, update the decoder/writer first, add a
compatibility regression, update the detailed contract document, and then
change this table. A release must include the exact source revision and test
evidence for any newly compatible version.
