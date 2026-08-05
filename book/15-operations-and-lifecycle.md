# 15. Declarative Systems, Updates, and Operations

The project includes both low-level mechanisms and operator-facing lifecycle tools.

## Declarative configuration

`synos-declarative` parses signed declarative configuration, validates schema and compatibility, produces structured diffs, and atomically activates a new system root. Configuration can include cluster identity, discovery, membership, quorum, transport, security, resources, federation, node overrides, policy inheritance, and maintenance windows.

The activation pattern is:

```text
parse -> validate -> diff -> sign -> quorum acknowledge -> activate
                                                   |
                                      failure -> rollback
```

## Package lifecycle

`synos-pkg` resolves packages by immutable content ID. `synos-auditd` watches package security and obsolescence. `synos-update` validates signed update manifests, compatibility, staged activation, rollback, and crash recovery.

Do not update the running root in place. Stage a root, validate it, publish it, and keep the old generation until the new one proves healthy.

## Backup and disaster recovery

`synos-backup` streams live SynFS versions from pinned checkpoints. Remote storage in `synos-storaged` models capability-gated NAS and enterprise storage operations. Recovery must preserve versions, checksums, authorization, and partial-failure evidence.

## Cache and storage services

`synos-kvd` provides a native bounded in-memory key-value cache. It must define eviction, persistence, restart, stale generation, and capacity behavior. `synos-storaged` provides remote storage service models with capability and lifecycle controls.

## Power and RAS

`synos-power` handles ACPI and thermal lifecycle. `synos-ras` handles reliability, availability, serviceability records, hardware error prediction, alert deduplication, and fault-injection behavior.

## Developer tools

`cargo-synos` provides Cargo integration for repository workflows. `synos-debug`, `synos-inspect`, `synos-top`, and `synos-replay` form a practical operations kit:

```text
inspect -> observe -> trace/replay -> debug -> recover
```

The `clients/apple` package provides Swift transport, model, and control UI
types for a native Apple client. `synos-posix-compat` provides a bounded
compatibility surface for software that needs familiar POSIX/Linux-shaped
operations without moving the kernel back toward a monolithic Unix design.

## Rust service toolchain

Ring 3 services target `targets/x86_64-unknown-synos.json` or
`targets/aarch64-unknown-synos.json` as position-independent static images.
`synos-posix-compat` translates a bounded file-descriptor and Linux syscall
surface into SynFS capabilities and shared-buffer descriptors. It supports
familiar operations such as open, close, read, write, seek, clocks, getpid, and
exit without giving the process an ambient Unix namespace.

`cargo-synos` can build Ring 3 programs, build the required `core` and `alloc`
from pinned `rust-src`, and create signed `.synpkg` bundles:

```sh
cargo install --path tools/cargo-synos
cargo synos package --bin example-service --target x86_64 \
  --release --key package-signing.key --output example-service.synpkg
```

Dependencies are pinned by 64-character SHA-256 IDs. The package daemon checks
the signing key, signature, payload digest, dependencies, and complete system
manifest before activation.

## USB and dual boot

The bootstrap supports separate experimental UEFI and legacy BIOS USB paths.
UEFI copies the loader to the exact fallback path:

```text
EFI/BOOT/BOOTX64.EFI
```

Legacy BIOS writes the complete `build/bios/synos-bios.img` to the whole drive,
not to one partition. The UEFI loader can also present SynOS, Windows Boot
Manager, and GRUB choices, and the portable image can be placed on an existing
filesystem and started through a GRUB loopback entry without repartitioning.

USB writing erases the selected device. Verify the device name twice before
using `dd` or an equivalent tool.

## Read-only host filesystems

The host filesystem service scans MBR/GPT and reads FAT32, Ext4, and NTFS
volumes through a positional `ReadAt` contract. It supports long names, extent
trees, MFT data runs, sparse/resident/non-resident data, and bounded directory
indexes. It does not write, replay Ext4 journals, or decode compressed,
encrypted, or deduplicated NTFS files.

## Multi-tenant quotas

Every live kernel capability can own fixed resource state. Administrators with
`CONTROL` can configure token buckets for IPC messages, page faults, and
allocation bytes, plus an outstanding-memory ceiling. Failed sends or
allocations refund their charge. Fabric managers can also apply per-node/channel
CXL bandwidth QoS, returning a busy result when one tenant would consume another
tenant’s budget.

## Qualification

The `platforms` directory defines profiles for consumer systems, QEMU/KVM clusters, CXL racks, and GPU fabrics. `scripts/qualify-platform.sh` checks captured evidence such as:

- boot markers;
- hardware inventory;
- heartbeat and page movement;
- failover completion;
- CXL Type-3 and HDM evidence;
- PCIe/NVLink and VRAM pool evidence.

## Docker

The Docker image bundles the build and QEMU environment. A single node can expose serial output and VNC. Cluster mode launches multiple q35 guests with unique displays and logs.

```sh
docker build -t synos:latest .
docker run --rm -it -p 5900:5900 synos single
docker compose --profile cluster up synos-cluster
```

This is useful for onboarding: the host needs Docker instead of the complete native toolchain.
