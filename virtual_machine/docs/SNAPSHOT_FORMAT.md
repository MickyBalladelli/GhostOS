# Snapshot format and upgrade path

`VmSnapshot` is a portable checkpoint for guest CPU, RAM/MMU, interrupt,
APIC, and BIOS state. Host handles, device queues, timers, disks, and network
backends stay outside the checkpoint; see [the state inventory](SNAPSHOT_STATE_INVENTORY.md).

## Wire formats

Every snapshot starts with the eight-byte magic `SYNOVM01`, followed by a
little-endian `u32` format version and a little-endian `u64` RAM size.

| Version | Compatibility | Feature word |
| --- | --- | --- |
| 1 | Read and write | Absent; treated as `SnapshotFeatures::ALL` |
| 2 | Read and write | Little-endian `u64` immediately after the version |

Version 2 rejects unknown feature bits and rejects snapshots missing any
currently required state component. All length-prefixed arrays are checked
against the remaining input, `MAX_ITEMS`, and the RAM/whole-snapshot byte
limits before allocation.

## Negotiation

Migration protocol `SYNOMIG2` exchanges a protocol version and a
`SnapshotSchema` containing a supported version range plus feature bits. The
highest overlapping version is selected. A migration fails when ranges do not
overlap or the feature intersection lacks a required component.

`SYNOMIG1` remains a receive-only compatibility path for older senders. It
accepts one version-1 snapshot and does not perform a feature handshake.

## Upgrade path

1. Load the old checkpoint with `VmSnapshot::from_bytes`; malformed,
   oversized, unknown-version, and unknown-feature input is rejected.
2. Negotiate with `SnapshotSchema::negotiate_with`.
3. Convert the validated checkpoint with `VmSnapshot::convert_to_schema`.
   Version 1 has implicit `ALL` flags; conversion to version 2 writes the
   explicit flags word. Current versions have the same state payload, so no
   guest state migration is needed.
4. Save the converted checkpoint and verify it by reopening it with
   `VmSnapshot::load` before using it for restore or migration.

When a future format changes the state payload, add a new version and decoder,
keep the old decoder for at least one compatibility window, add a conversion
step here, and update the schema matrix and state inventory in the same
change. Never reinterpret an old field in place or silently ignore a feature
bit.

The migration TCP path is not authenticated or encrypted. Use an authorized,
protected transport until the separate migration transport-security work is
complete.
