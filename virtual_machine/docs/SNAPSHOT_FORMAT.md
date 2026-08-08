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

The raw `SYNOVM01` payload is a format-conversion primitive, not a trusted
checkpoint. Persistent snapshots use the `SYNOSIG1` envelope:

| Field | Size |
| --- | ---: |
| magic, version, algorithm, reserved bytes | 16 bytes |
| key identifier | 16 bytes |
| payload length | 8 bytes |
| `SYNOVM01` payload | variable |
| HMAC-SHA256 tag | 32 bytes |

The tag covers every envelope byte before the tag. Decoding verifies the tag
before passing the payload to the bounded snapshot decoder.

## Negotiation

Migration protocol `SYNOMIG3` exchanges a protocol version and a
`SnapshotSchema` containing a supported version range plus feature bits. The
highest overlapping version is selected. A migration fails when ranges do not
overlap or the feature intersection lacks a required component.

`SYNOMIG1` and `SYNOMIG2` are legacy unauthenticated protocols and are rejected
by the migration listener. Convert old version-1 payloads offline, wrap them in
an authenticated snapshot envelope, then migrate with `SYNOMIG3`.

## Upgrade path

1. Load the old checkpoint with `VmSnapshot::from_bytes`; malformed,
   oversized, unknown-version, and unknown-feature input is rejected.
2. Negotiate with `SnapshotSchema::negotiate_with`.
3. Convert the validated checkpoint with `VmSnapshot::convert_to_schema`.
   Version 1 has implicit `ALL` flags; conversion to version 2 writes the
   explicit flags word. Current versions have the same state payload, so no
   guest state migration is needed.
4. Save the converted checkpoint with `VmSnapshot::save_authenticated` and
   verify it by reopening it with `VmSnapshot::load_authenticated` before using
   it for restore or migration.

When a future format changes the state payload, add a new version and decoder,
keep the old decoder for at least one compatibility window, add a conversion
step here, and update the schema matrix and state inventory in the same
change. Never reinterpret an old field in place or silently ignore a feature
bit.

Snapshot envelopes and migration frames use HMAC-SHA256 with a configured
32-byte shared key. The migration tag covers the negotiated schema, payload
length, and authenticated snapshot bytes. Authentication does not encrypt the
TCP path or provide replay protection; use an authorized protected transport
and rotate keys through an external key-management process.
