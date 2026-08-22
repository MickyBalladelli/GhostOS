# 7. GhostFS, Persistence, and Storage

GhostFS is the day-one filesystem core. It is `no_std`, fixed-capacity, and built around immutable Copy-on-Write metadata.

## The filesystem shape

GhostFS stores metadata in B+tree blocks. A write produces a new version rather than mutating the only copy of the old tree.

```text
generation 10: root -> A -> B -> C
write note:    root -> A -> B' -> C'
                         ^ unchanged blocks remain shared
```

If a later operation fails, generation 10 is still valid. If it succeeds, generation 11 becomes the published root.

## File versions

The same file can be addressed by name, exact version, or latest version:

```text
/notes.txt       latest
/notes.txt;0     base/latest according to the filesystem rule
/notes.txt;1     exact immutable version
```

The important user promise is that a version selector never silently changes meaning because a later write occurred.

## Blocks, retention, and collection

File data is split into content-matched blocks. Unchanged tails can be shared. `SynfsPurged` applies bounded retention work, then mark-and-sweep collection reclaims tombstoned data and abandoned CoW branches.

The collector is part of the storage contract. It must not reclaim a block still pinned by a snapshot, checkpoint, backup, mapped record file, or active generation.

## Persistence path

The filesystem appears through several layers:

```text
GhostFS core
  -> ghostos-fsd namespace service
  -> runtime ABI and IPC
  -> ghostos-shell commands
  -> client SDK / HTTP / remote terminal
```

Tests should exercise both direct GhostFS behavior and at least one public boundary.

The write, commit, rename, flush, sync, cache, block-device, and power-loss
rules are defined in [`docs/durability.md`](../docs/durability.md).

## Storage pools

`StoragePoolAdmin` combines NVMe namespaces, CXL persistent memory, and network block targets into pools. Pools can be striped or separated by failure domain. Lifecycle states include registration, online use, draining, failure, degraded operation, and safe detach.

Draining blocks new allocations. Detach refuses to proceed while a pool still owns leases or data that has not been rebuilt.

## Backup and restore

The backup worker pins a GhostFS checkpoint and streams live file versions into a checksummed `SYNBACK1` archive through bounded cooperative polls. It releases the pinned root only after the caller commits the finished backup.

Recovery backups use the same pinned-root boundary with keyed, encrypted,
content-addressed chunks, resumable object uploads, catalog retention and
legal holds, and a transactional restore verifier. Restore reports include RPO,
RTO, bytes transferred, bootability, and every skipped object with its reason.

The full compatibility and recovery matrix is in
[`docs/persistence-compatibility.md`](../docs/persistence-compatibility.md).
`SYNBACK1` remains a streaming writer format; recovery manifests and encrypted
objects are the restore input for the new recovery API.

```text
checkpoint -> stream -> checksum -> caller commits -> release checkpoint
                         |
                         +-> failure: keep source root, discard partial archive
```

## Host filesystem discovery

`ghostos-host-filesystems` provides read-only discovery for Ext4, FAT32, and NTFS. Discovery validates partition metadata, logical block sizes, offsets, lengths, and supported filesystem structures. It does not turn a host filesystem into an unrestricted writable mount.

## Package data

`ghostos-system-model` and `ghostos-pkg` use content-addressed package objects. A package root binds logical names to SHA-256 IDs. Dependency resolution and activation use immutable digests. Package payloads live as GhostFS objects; the system model stores metadata and activation state.

## RMS and key/value data

RMS record files sit on GhostFS versions. Sequential and indexed records use caller-provided scratch buffers and stable selectors. `ghostos-rms` adds create, read, insert, update, delete, record locks, and a fixed-capacity embedded key-value database.

Transactions stage writes and publish one new root only if all operations succeed:

```text
old root -> put A -> delete B -> put C -> commit
                                      |
                         any failure -> restore old root
```

## Easy example: persistent note

```text
CREATE /DATA/notes/today
EDIT /DATA/notes/today
TYPE /DATA/notes/today
SNAPSHOT /DATA/notes/today
```

The shell command names are the human layer. Under them are generations, blocks, locks, and checksums that make the note recoverable.
