# Snapshot format and upgrade path

`VmSnapshot` is a portable checkpoint for guest CPU, RAM/MMU, interrupt,
APIC, and BIOS state. Host handles, device queues, timers, disks, and network
backends stay outside the checkpoint; see [the state inventory](SNAPSHOT_STATE_INVENTORY.md).
Networking restart state follows one rule: the VM configuration rebuilds NIC
topology and MAC identity, while the network service or fixture separately
recovers DHCP lease records, routes, DNS, neighbors, and relative timer
deadlines. Pending packets, queue depth, carrier state, and backend handles
must be re-established after restore.
`VmSnapshot::restore_into_with_report` and
`Vm::restore_snapshot_with_report` report what was restored, what must be
rebuilt by VM construction, and what remains excluded. The CLI prints this
boundary during restore. For exact continuation, restore into a fresh VM made
from the same configuration and reconnect the reported external resources.

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
highest overlapping version is selected. Both peers prove possession of the
shared key with mutual nonces before checkpoint bytes are accepted. A migration
fails when ranges do not overlap or the feature intersection lacks a required
component.

Migration reads and writes have a ten-second timeout. The receiver rejects
payloads above its 8 GiB pre-allocation migration cap, rejects checkpoints older than
24 hours or more than five minutes in the future, and records accepted
checkpoint identities in `.ghostos-vm-migration-replay` beside the destination.
The sender also bounds and regular-file-checks the source before reading it.
The bounded ledger is private, locked across processes, and synced before
publication; it rejects duplicate deliveries during the freshness window and
rolls back a reservation when publication fails.
Received checkpoints are written to a unique temporary file in the destination
directory, synced, atomically published, and followed by a directory sync.
The published file is reopened and authenticated before the receive succeeds;
an existing destination is retained until that verification completes and is
restored if publication fails.

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
length, freshness metadata, both nonces, and authenticated snapshot bytes.
Authentication does not encrypt the TCP path. Direct TCP over an untrusted
network is forbidden. Use mutually authenticated TLS, an authorized VPN, or an
SSH tunnel and pass `--secure-transport` only after that protection exists.
Every migration also requires an allow-listed peer key ID and a private durable
JSONL audit log. Rotate keys through an external key-management process.
