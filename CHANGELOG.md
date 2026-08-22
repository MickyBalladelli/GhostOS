# Changelog

This file records changes that matter to users, guest drivers, image owners,
and release operators. Keep the newest work under `[Unreleased]`; move it to a
version heading when publishing a release.

## Changelog rule

Every change under `virtual_machine/src` that can alter guest-visible CPU,
memory, firmware, device, interrupt, terminal, boot, CLI, or network behavior
must add one entry under `### Guest-visible`. Every snapshot or migration wire
change must add an entry under `### Snapshot and migration`. Every RAW, fixed
VHD, QCOW2, system-disk header, manifest, lock, or persistence-layout change
must add an entry under `### Disk formats`.

When reviewing a patch, enforce the affected categories with
`python3 scripts/validate-changelog.py --changed-file PATH` for each changed
VM source path. The release gate always checks the changelog structure, and a
release review must run the changed-file form against the release diff.

Each entry must state:

- `Impact`: what a guest, operator, or image owner observes;
- `Compatibility`: old/new versions, device models, firmware modes, and CLI
  combinations that remain valid;
- `Evidence`: the executed test/evidence ID or a precise reason for a skip;
- `Migration`: required for snapshot, migration, and disk-format entries; say
  how old state is upgraded, rejected, or safely retained.

Documentation, test-only, and host-internal changes belong under
`### Tooling and documentation` and must say when guest compatibility is
unchanged. Do not silently rewrite an old entry: append a new entry and keep
the released text immutable.

## [Unreleased]

### Guest-visible

- Renamed the product from SynOS to GhostOS, including the kernel bootstrap
  string, shell prompt (`GHOSTOS::ROOT`), default hostname, CLI binary
  `ghostos-vm`, and UEFI loader `ghostos-loader`. Impact: operator-facing
  names and guest serial/VGA branding change. Compatibility: CPU, firmware,
  and device models are unchanged. Evidence: workspace compile after rename.

### Snapshot and migration

- Snapshot envelope magic `SYNOSIG1` and HMAC domains
  `SYNOS-MIGRATION-HMAC-SHA256-V3` / `SYNOS-MONITOR-HMAC-SHA256-V1` are
  unchanged. Impact: existing authenticated snapshots and live migration
  continue to verify. Compatibility: no wire-format bump. Evidence: magic
  strings retained in `snapshot.rs`, `migration.rs`, and `control.rs`.
  Migration: none; readers and writers still use the SynOS fourccs.

### Disk formats

- Writable disk ownership markers are now `<image>.ghostos.lock`. Impact:
  new VM runs create GhostOS lock files. Compatibility: inspect/recover still
  accept leftover `<image>.synos.lock` files; new locks are only
  `.ghostos.lock`. Evidence: `DiskImage` lock helpers. Migration: recover or
  delete stale `.synos.lock` files, then start the VM so it can create a
  `.ghostos.lock`. System-disk header `SYNOSDSK` and GhostFS volume magics
  stay as previously published fourccs.

### Tooling and documentation

- Renamed crates, scripts, Docker images, Rust targets, and docs from SynOS
  to GhostOS (`ghostos-*`, `x86_64-unknown-ghostos`, `cargo ghostos`).
  Impact: host tooling and package names change. Compatibility: guest
  snapshot, disk header, and RPC `SYRP` bytes are unchanged. Evidence:
  workspace package rename.
- Added the VM public contract, compatibility matrices, and release artifact
  manifest process. Impact: documentation and packaging only. Compatibility:
  guest, snapshot, and disk behavior unchanged. Evidence: documentation review.
- Added assigned fuzz targets, retained seed corpora, and deterministic failure
  replay metadata for untrusted parser boundaries. Impact: tooling and
  documentation only. Compatibility: guest, snapshot, and disk behavior
  unchanged. Evidence: fuzz inventory validation.
- Added a required release compatibility proof for upgrade and rollback paths.
  Impact: release tooling now rejects packages without two-way compatibility
  evidence. Compatibility: guest, snapshot, disk, and package formats are
  unchanged. Evidence: release compatibility validator.
- Added the stable user-space SDK compatibility policy and exposed the SDK
  contract to Rust and Swift clients. Impact: SDK releases now have an explicit
  source, wire, deprecation, and migration contract. Compatibility: existing
  SDK and `SYRP` v1 behavior is unchanged. Evidence: compatibility contract
  registry and documentation review.

## Release format

Use a dated version heading such as `## [0.2.0] - 2026-08-09`. Keep the four
category headings in every release section. A release is not complete until
the packaged artifact contains this file and its recorded SHA-256 digest.
