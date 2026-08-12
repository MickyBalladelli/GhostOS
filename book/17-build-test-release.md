# 17. Build, Test, Fuzz, and Release

The repository treats evidence as part of delivery. A green local command is useful; a reproducible result with a stable tier and artifact is better.

## Toolchain

`rust-toolchain.toml` requests stable Rust with `rust-src` and `llvm-tools`, plus bare-metal targets:

- `x86_64-unknown-none`;
- `x86_64-unknown-uefi`;
- `aarch64-unknown-none`;
- `riscv64gc-unknown-none-elf`.

Install `clang` for BIOS assembly and image creation. Optional tiers need QEMU, `cargo-fuzz`, `cargo-llvm-cov`, and `cargo-mutants`.

## Build commands

Build and test the main workspace:

```sh
cargo test
```

Build a BIOS image:

```sh
./scripts/build-bios-image.sh
```

Build the whole practical path:

```sh
./scripts/build-and-test.sh
```

Build the UEFI loader:

```sh
cargo uefi --release
```

Build a portable image:

```sh
cargo uefi --release
./scripts/build-portable-image.sh
```

## Test tiers

The deterministic runner is:

```sh
./scripts/test-all.sh
```

It records separate results for host, unit, integration, workspace, VM, VM quality, performance, and recovery tiers.

Full validation is opt-in:

```sh
SYNOS_FULL_VALIDATION=1 ./scripts/full-validation.sh
```

It adds QEMU, cluster, hardware acceleration, fuzzing, coverage, mutation testing, soak runs, reproducibility, dashboard, and release-gate checks. Missing prerequisites become explicit `skipped` results.

## Evidence

Evidence is stored under `build/test-evidence/<run-id>/`. Each tier gets:

```text
metadata.json
stdout.log
stderr.log
result.json
```

The dashboard summarizes pass, fail, and skip state. The release gate rejects unexpected failures, invalid result JSON, bad revision provenance, panic markers, missing clean QEMU boot evidence, and unexplained image provenance.

Full validation also writes `evidence-manifest.json` and the detached
`evidence-manifest.json.sha256` digest at the evidence root. The manifest
records every evidence file's size and SHA-256 digest, result state, and Git
revision. Full validation verifies the manifest before the release gate; a
changed, missing, or unexplained evidence record fails validation.

Package the VM release only after the evidence run passes:

First build the release report. It joins correctness results with the latency
percentiles, declared resource ceilings, fault-recovery RTO samples, supported
CPU scale tiers, and known limits. Fault recovery input must be a signed or
otherwise retained JSON report with `revision`, `samples`, `fault`, `workflow`,
`observed_rto_ms`, and `rto_budget_ms` fields:

```sh
python3 scripts/release-report.py --write \
  --evidence-dir build/test-evidence/<run-id> \
  --recovery-report build/recovery/fault-recovery-report.json \
  --output build/release/release-report.json
```

The command refuses failed or inconclusive correctness evidence, missing
p50/p95/p99 data, missing resource ceilings, over-budget recovery samples, or
an empty known-limits section. Verify it independently with:

```sh
python3 scripts/release-report.py --check \
  --evidence-dir build/test-evidence/<run-id> \
  --report build/release/release-report.json
```

Then package the VM release:

```sh
python3 scripts/package-vm-release.py \
  --output build/release/synos-vm.tar.gz \
  --artifact target/release/synos-vm \
  --artifact build/bios/synos-bios.img \
  --artifact target/x86_64-unknown-uefi/release/synos-loader.efi \
  --evidence-dir build/test-evidence/<run-id> \
  --release-report build/release/release-report.json \
  --attestation-dir build/release/attestations \
  --firmware bios --firmware uefi
```

The archive contains the verified `release-report.json`. Its manifest records
the report digest alongside the other release inputs. The report records
SHA-256 digests, source revision, firmware
coverage, default device topology, every executed evidence record, and known
host limitations. It also carries the exact changelog and its digest. Failed
evidence prevents packaging; skipped evidence stays in the manifest with its
prerequisite reason.

Create the attestation directory before packaging. The release signer keeps
the private key outside the repository and provides a separately produced copy
of every artifact for the reproducibility comparison:

```sh
python3 scripts/release-attestations.py --write \
  --output-dir build/release/attestations \
  --signing-key /secure/release-key.pem \
  --artifact synos-vm=target/release/synos-vm \
  --artifact synos-bios=build/bios/synos-bios.img \
  --artifact synos-loader=target/x86_64-unknown-uefi/release/synos-loader.efi \
  --reproducible-artifact synos-vm=/independent-build/synos-vm \
  --reproducible-artifact synos-bios=/independent-build/synos-bios.img \
  --reproducible-artifact synos-loader=/independent-build/synos-loader.efi
```

The generator emits a CycloneDX SBOM, Cargo.lock dependency provenance,
compiler/toolchain identity, tracked-source and release-configuration digests,
and a byte-for-byte reproducibility result inside each signed statement.
Packaging runs the verifier with the public key and rejects missing coverage,
changed artifact bytes, invalid signatures, or an unverified reproducibility
comparison. Verify an attestation directory independently with:

```sh
python3 scripts/release-attestations.py --check \
  --attestation-dir build/release/attestations \
  --artifact synos-vm=target/release/synos-vm \
  --artifact synos-bios=build/bios/synos-bios.img \
  --artifact synos-loader=target/x86_64-unknown-uefi/release/synos-loader.efi
```

If the `.tar.gz` archive itself is published as an artifact, run the same
generator once more over that archive and its independently reproduced copy;
publish that second attestation directory beside the archive.

For a release diff, run the changelog validator once per changed VM source
path, for example:

```sh
python3 scripts/validate-changelog.py \
  --changed-file virtual_machine/src/devices/storage/disk_image.rs
```

## Test contract

Every feature needs direct behavior, boundary/error, integration, and end-to-end evidence when it crosses a process, device, boot, persistence, or cluster boundary. Persistent and distributed features need restart, corruption, timeout, duplicate, and partial-failure cases.

The VM inventory uses stable IDs for source modules, public APIs, devices, boot paths, and test files. Each ID resolves to an executed evidence record containing its command, revision, host, firmware, CPU count, image digest, and result state. `scripts/validate-vm-quality.py` checks that every VM source module and public API has a named test.

## Fuzzing

The fuzz workspace targets parsers and untrusted bytes. VM-specific targets cover:

- `vm-decoder` — arbitrary instruction streams;
- `vm-devices` — PCI and port-device boundaries;
- `vm-images` — raw/VHD/QCOW2 image parsing.

Other targets cover paths, SynFS volumes, filesystem operations, mounts, HTTP, and scripts.

Every fuzz crash should become a deterministic regression test with the original seed or corpus artifact.

## Mutation and coverage

`scripts/coverage.sh` produces workspace and per-crate reports. Per-crate thresholds stop a large healthy crate from hiding a small untested crate.

`scripts/mutation.sh` runs `cargo-mutants` across high-risk boundaries including status, auth, filesystem, SynFS, HTTP, and the VM.

## Cross-platform validation

Local validation runs deterministic tests on Linux, macOS, and Windows through the repository scripts. Fuzzing and full validation are opt-in. Hardware, QEMU, cluster, and long-running tiers remain explicit so their environmental requirements are visible.

## The rule for contributors

When changing code:

```text
change -> test -> inventory -> evidence -> document
```

When fixing a bug:

```text
reproduce -> add deterministic regression -> fix -> preserve evidence
```
