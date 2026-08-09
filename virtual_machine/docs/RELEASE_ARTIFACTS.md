# VM release artifacts

Use `scripts/package-vm-release.py` after a validation run. It creates one
reproducible `tar.gz` archive and refuses to package failed evidence:

```text
release archive/
├── release-manifest.json
├── CHANGELOG.md
├── artifacts/
│   ├── synos-vm
│   ├── synos-bios.img
│   └── synos-loader.efi
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
| `device_topology` | Default guest device names, transports, addresses, and interrupt vectors |
| `test_evidence` | Evidence JSON paths, digests, tier, test ID, state, command, firmware, host, and skip reason |
| `known_host_limitations` | Hardware acceleration, terminal, networking, migration, and optional-test limits |

The package command requires explicit `--artifact`, `--evidence-dir`, and one
or more `--firmware` values. Artifact paths are stored only by basename in the
manifest; host paths do not leak into the release metadata. Evidence files are
copied under `evidence/` and their paths are relative to the supplied evidence
directory.

Skipped evidence is allowed only when its record includes a prerequisite or
reason. Failed evidence is never releaseable. Consumers must verify the
archive and each listed SHA-256 digest before installation, then verify the
source revision and firmware mode against their deployment record.

The topology describes the default `VmConfig`; custom serial ports, attached
disks, firmware application images, and host backends must be recorded by the
operator as deployment-specific metadata.
