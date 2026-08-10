# Persistent format compatibility

This is the compatibility map for bytes that SynOS writes to durable storage.
The Rust reader is the final authority. This document records current reader
behavior and safe operator action; it does not promise compatibility that the
code does not implement.

## Rules

1. Unknown magic, version, feature, checksum, length, or required field is a
   corrupt artifact. Do not guess or reinterpret it.
2. A format upgrade adds an explicit decoder and converter. It never changes
   the meaning of an old field in place.
3. Downgrade is export/import into a volume or artifact made by the older
   writer. No format below has an in-place downgrade contract.
4. Backups copy a committed generation or a pinned checkpoint. A partial
   stream, temporary file, or unverified signed object is not a backup.
5. Recovery evidence records source revision, exact command, host, start and
   end time, result, and artifact checksum. Test names below are regression
   coverage; a release still needs the evidence artifact described in
   [`docs/testing.md`](testing.md).

## SynFS and record data

| Artifact | Current format | Backup and migration | Downgrade and recovery |
| --- | --- | --- | --- |
| SynFS volume | `SYNFSVOL`, volume version `3`; `SYNFSMAP`; tree blocks `SYNT` version `1`. Two committed banks, checksummed type maps, and checksummed blocks. | Back up only after a flush, or stream a pinned root with `SYNBACK1`. There is no v1/v2 volume converter in the tree. | Older readers reject v3. Create a target volume and explicitly import data. `recovery_uses_newest_complete_generation_and_rejects_partial_objects`, `recovery_reports_corruption_when_no_complete_generation_remains`, and `format_generation_selection_and_checksum_validation` cover selection and corruption. |
| RMS record image | `SYNRMS01`; fixed 24-byte header, descriptor, and bounded length-prefixed records. It has no independent version field. | Store the record image as a normal SynFS file and back up that file. Descriptor changes require a record-level converter. | Treat layout changes as incompatible. Validate the complete image before use; never parse a newer image as an older one. |
| SynFS backup stream | Header `SYNBACK1`, format `1`; trailer `SYNBEND1`. Header includes checkpoint and generation; file entries include version, size, checksum, and creation time. | The writer pins a checkpoint and polls with a byte budget. The current crate has a writer, not a restore decoder or mountable archive reader. A `SYNBACK1` stream is therefore not yet a disaster-restore input by itself. | Do not claim restore or downgrade compatibility until a decoder, checksum verification, and target-volume importer exist. `backup_streams_a_pinned_snapshot_and_releases_it` and `backup_rejects_invalid_budget_and_sink_failure_without_losing_checkpoint` cover writer safety. |
| Legacy kernel shell store | `SYNFS001`, version `1`, bounded to 32 KiB in the kernel persistence port. | It is legacy bootstrap state, not a SynFS volume. Copy it only as a whole validated blob. No converter exists. | Invalid or unknown data is ignored and the built-in filesystem remains. Upgrade by exporting through the legacy API and recreating state; no in-place downgrade. |

## VM disks, checkpoints, and replay

| Artifact | Current format | Backup and migration | Downgrade and recovery |
| --- | --- | --- | --- |
| Raw/VHD/QCOW2 disk image | RAW has no magic. Fixed VHD uses `conectix`; QCOW2 uses `QFI\xfb`. The VM accepts bounded subsets and validates geometry, checksums, features, and backing-file restrictions. | Preserve original image bytes and format. A VM snapshot does not include disk bytes. Flush and sync the controller before copying. | The VM does not convert disk formats. Use an offline image converter, then reopen and validate capacity and geometry. Never write a VHD or QCOW2 image as RAW. |
| SynOS system disk metadata | System-disk format `1`; header `SYNOSDSK`, manifest `SYNMANIF`, settings `SYNSET01`. Two manifest slots are published after payload sync. | Back up the image and metadata together. A snapshot restore does not restore the external system disk. | Unknown versions are rejected. Readers select the newest complete, checksum-valid slot and fall back to the other slot after interruption. Downgrade requires a separately generated older system disk. |
| Guest persistence tail | `SYNOPS01`, version `1`, in the final 64 KiB of a disk image. Payload length and checksum are validated. | Copy the whole disk or preserve this tail with its image. It is separate from normal guest sectors. | Unknown or invalid metadata is treated as empty state. No converter or in-place downgrade exists. |
| VM snapshot | Raw payload `SYNOVM01`, versions `1..2`; authenticated envelope `SYNOSIG1`, version `1`, HMAC-SHA256. | Persistent or remote checkpoints must use the authenticated envelope. Restore uses matching RAM size and rebuilds or excludes host-owned state listed in [`SNAPSHOT_STATE_INVENTORY.md`](../virtual_machine/docs/SNAPSHOT_STATE_INVENTORY.md). | v1 is readable and converts to v2 with implicit `ALL` feature flags. Future versions need an explicit converter and schema update. `SYNOMIG1` and `SYNOMIG2` are rejected; migration accepts `SYNOMIG3` only. Detailed rules are in [`SNAPSHOT_FORMAT.md`](../virtual_machine/docs/SNAPSHOT_FORMAT.md). |
| VM replay trace | `SYNVMRP1`, version `1`, bounded to 128 MiB and one million events. | It is diagnostic input, not VM restore state. Copy only after `sync_all`; validate sequence numbers and payload limits on load. | No migration or downgrade exists. A version mismatch or trailing bytes makes the trace corrupt. |
| Compiler toolchain archive | `SYNTOOL1`, version `1`, nested inside a signed package bundle. Asset paths and content IDs are checked. | Back up the signed bundle and its trust material. Rebuild a toolchain archive when the archive version changes. | No in-place downgrade. An older compiler accepts only its known archive version and must receive a separately built compatible bundle. |

## Packages and configuration

| Artifact | Current format | Backup and migration | Downgrade and recovery |
| --- | --- | --- | --- |
| Package bundle | `SYNBNDL1`, version `1`; content IDs, dependency list, key ID, and HMAC signature. | Back up immutable bundle bytes and the trusted key identifier set. Reverify signature, payload hash, dependencies, and complete root during restore. | Unknown versions, lengths, keys, hashes, and signatures are rejected. No in-place downgrade; install a separately signed v1 bundle. `signed_bundle_round_trip_detects_tampering_and_trust_failures`, `supply_chain_checks_signature_hash_and_dependency_bounds`, and `package_hash_mismatch_cannot_become_an_instantiation_receipt` are the recovery gates. |
| Application bundle | `SYNAPP01`, version `1`, containing a v1 package bundle and schema-bearing application metadata. | Back up the outer bundle and inner package together. Validate both signatures and the metadata/package content IDs before activation. | Unknown outer or inner versions are rejected. Rebuild an older application bundle with an older toolchain; do not edit metadata in place. |
| Declarative system configuration | Text schema `1`, signed and activated as a new SynFS generation. | Back up the signed source, signature, canonical digest, and activated generation. Restore by validation and atomic activation, never by editing the live root. | Schema mismatch is rejected. `signed_activation_is_targeted_atomic_and_rollback_safe` covers rollback; conversion to an older schema must be an explicit source-level exporter. |

## Cluster and service state

All rows in this section are stored as SynFS files and must be committed in a
SynFS transaction. They use exact version checks and checksums where shown.

| Artifact | Current format | Backup, migration, and downgrade |
| --- | --- | --- |
| Cluster metadata | `/system/cluster/metadata.dat`, `SYNCLID1`, version `1`, checksum. | Back up with the containing SynFS generation. Decode and checksum-check before restore. No converter or in-place downgrade exists. |
| Membership registry | `/system/cluster/membership.dat`, `SYNMEMB1`, version `1`, fixed records and checksum. | Preserve cluster ID, epoch, term, commit index, and next index together. Reject stale or mismatched cluster state; rebuild through a quorum-approved migration. No in-place downgrade. |
| Bootstrap state | `/system/cluster/bootstrap.dat`, `SYNBOOT1`, version `1`, signed credentials and checksum. | Back up only with the trust root and credential epoch. Revalidate signature, expiry, scope, and checksum. Rotate credentials through a new v1 record; never downgrade credentials by copying old bytes. |
| Admission audit | `/system/cluster/admission-audit.dat`, `SYNADIT1`, version `1`, bounded records and checksum. | Preserve it as evidence with the cluster generation. Corruption is a hard evidence failure, not an empty journal. No migration or downgrade exists. |
| Mount catalog | `SYNMNT01`, catalog version is a monotonic field rather than a format version, fixed 512-byte records. | Restore only after validating endpoints, protocol, options, and catalog generation. A layout change needs a new magic or explicit converter. `mount_catalog_round_trips_through_synfs_and_rejects_corruption` is the direct gate. |
| Observability journal record | `SLOG`, record version `1`, exactly 128 bytes with checksum. | Export complete records after flush. Replay only records that pass magic, version, field, and checksum checks. No migration or downgrade exists. |
| KVD state | `SYNKVD01`, bounded key/value entries and lengths. | Back up as a SynFS state file. Validate uniqueness, capacities, and complete consumption before loading. No converter or downgrade exists; discard invalid cache state rather than treating it as authoritative data. |
| Agent snapshot state | `SYNAGNT1`, version `1`, 56-byte header plus state checksum. | Restore only for the expected agent ID and exact payload checksum. The surrounding SynFS checkpoint keeps related files consistent. No in-place downgrade. |
| Crash flight recorder | `SYNREP01`, version `1`, `META` plus fixed 56-byte event files. | Preserve the receipt directory and SynFS generation as post-mortem evidence. Validate event kind, sequence, and bounded lengths. No restore or downgrade contract. |
| Core dump | `SYNCORE1`, version `1`, metadata plus page records. | It is post-mortem evidence, not executable state. Preserve metadata and all pages from one committed generation. The writer emits v1; no restore decoder or downgrade exists. |
| LLM inference state | `SYNR`, version `1`, bounded state record. | Back up the complete record with its model/content IDs. Revalidate the model binding before restore. No converter or downgrade exists. |

## Recovery evidence matrix

Before release, run the named checks and store the required evidence artifact:

| Area | Direct recovery checks |
| --- | --- |
| SynFS volume and retention | `cargo test -p synos-synfs --test coverage_59_5 recovery_uses_newest_complete_generation_and_rejects_partial_objects`; `cargo test -p synos-synfs --test coverage_59_5 recovery_reports_corruption_when_no_complete_generation_remains`; long-run retention test in the same file. |
| Backup stream | `cargo test -p synos-backup --test coverage_59_5 backup_streams_a_pinned_snapshot_and_releases_it`; sink failure and cancellation test. |
| Packages and activation | `cargo test -p synos-pkg --test coverage_59_5 signed_bundle_round_trip_detects_tampering_and_trust_failures`; `cargo test -p synos-declarative --test coverage_59_10 signed_activation_is_targeted_atomic_and_rollback_safe`. |
| VM snapshot and disk boundary | `cargo test -p synos-vm --test snapshot_terminal_network_10_5 snapshot_serialization_restore_chain_and_failures`; inspect the snapshot format and state inventory documents. |
| Cluster state | `cargo test -p synos-storaged --test coverage_59_5 mount_catalog_round_trips_through_synfs_and_rejects_corruption`; `cargo test -p synos-storaged --test coverage_59_5 cluster_metadata_is_generation_safe_and_persistent`; membership and bootstrap round-trip checks in the same test targets. |
| Replay evidence | `cargo test -p synos-replay --test coverage_59_9 crash_flight_recorder_preserves_bounded_evidence`; VM replay coverage in the VM test inventory. |

The current repository has no general restore decoder for `SYNBACK1`, no
in-place downgrade converter for SynFS or service-state blobs, and no promise
that a raw VM snapshot restores external disks. Those are explicit boundaries,
not compatibility claims.

## Deliberately outside this inventory

Ext4, FAT32, and NTFS are read-only host formats consumed by
`synos-host-filesystems`; SynOS does not write or migrate them. `SYNOMIG3`, IPC,
client, boot-info, guest-agent, and other protocol versions are wire contracts,
not durable backup formats. Their compatibility rules belong with their
protocol documentation and tests.
