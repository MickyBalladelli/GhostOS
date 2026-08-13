# VM release artifacts

Use `scripts/package-vm-release.py` after a validation run. It creates one
reproducible `tar.gz` archive and refuses to package failed evidence:

```text
release archive/
├── release-manifest.json
├── release-report.json
├── release-claims.json
├── upgrade-compatibility.json
├── CHANGELOG.md
├── artifacts/
│   ├── synos-vm
│   ├── synos-bios.img
│   └── synos-loader.efi
├── attestations/
│   ├── attestation-public-key.pem
│   ├── attestations-index.json
│   ├── attestations-index.json.sig
│   ├── synos-sbom.cdx.json
│   ├── dependency-provenance.json
│   ├── synos-bios.artifact.sig
│   ├── synos-loader.efi.artifact.sig
│   └── ...
└── evidence/
    └── <validation-run files>
```

The archive has manifest schema `1`. `release-manifest.json` contains:

| Field | Meaning |
| --- | --- |
| `product`, `version`, `revision` | VM package identity and source revision |
| `source_date_epoch` | Timestamp used for deterministic archive metadata |
| `firmware_modes` | Firmware modes covered by this artifact (`bios`, `uefi`) |
| `artifacts` | Each packaged file's name, size, and SHA-256 digest |
| `changelog` | The packaged changelog path and SHA-256 digest |
| `release_report` | The verified release report path and SHA-256 digest |
| `release_claims` | The verified claims manifest path and SHA-256 digest |
| `upgrade_compatibility` | The verified upgrade and rollback compatibility proof path and SHA-256 digest |
| `device_topology` | Default guest device names, transports, addresses, and interrupt vectors |
| `test_evidence` | Evidence JSON paths, digests, tier, test ID, state, command, firmware, host, and skip reason |
| `known_host_limitations` | Hardware acceleration, terminal, networking, migration, and optional-test limits |

The attestation directory contains a CycloneDX SBOM and dependency provenance
document. The provenance records the exact `Cargo.lock` digest and package
checksums. Signed in-toto statements embed both documents and bind their file
digests. Each
release artifact has a matching `<artifact>.artifact.sig` detached signature
over its raw bytes. Consumers must verify that signature with
`attestation-public-key.pem` before installing the image or boot artifact.

The package command requires explicit `--artifact`, `--evidence-dir`,
`--release-report`, `--release-claims`, `--upgrade-compatibility`, and one or
more `--firmware` values. The compatibility proof must cover both directions
of every listed boundary: the release accepts the previous version, and the
previous release accepts the new version for rollback. Every direction must
have passed JSON evidence in the supplied evidence directory.
Artifact paths are
stored only by basename in the
manifest; host paths do not leak into the release metadata. Evidence files are
copied under `evidence/` and their paths are relative to the supplied evidence
directory.

`release-report.json` uses the `synos-release-report` schema. It contains the
correctness result inventory, p50/p95/p99 latency summaries, resource
ceilings, measured fault-recovery RTO samples and percentiles, supported CPU
scale tiers, known limits, and source digests. Generate and verify it with
`scripts/release-report.py` before packaging. The report is bounded to 4096
correctness records, 1024 latency metrics, and 1024 recovery samples.

`release-claims.json` uses the `synos-release-claims` schema. Every controlled
claim term has a named workload, measured threshold and observation, host
configuration, and a SHA-256-pinned evidence artifact retained in the archive.
The validator also checks the current `[Unreleased]` changelog section for
unmatched controlled terms.

Skipped evidence is allowed only when its record includes a prerequisite or
reason. Failed evidence is never releaseable. Consumers must verify the
archive and each listed SHA-256 digest before installation, then verify the
source revision and firmware mode against their deployment record.

The topology describes the default `VmConfig`; custom serial ports, attached
disks, firmware application images, and host backends must be recorded by the
operator as deployment-specific metadata.
