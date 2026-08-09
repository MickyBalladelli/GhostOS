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

- None yet.

### Snapshot and migration

- None yet.

### Disk formats

- None yet.

### Tooling and documentation

- Added the VM public contract, compatibility matrices, and release artifact
  manifest process. Impact: documentation and packaging only. Compatibility:
  guest, snapshot, and disk behavior unchanged. Evidence: documentation review.

## Release format

Use a dated version heading such as `## [0.2.0] - 2026-08-09`. Keep the four
category headings in every release section. A release is not complete until
the packaged artifact contains this file and its recorded SHA-256 digest.
