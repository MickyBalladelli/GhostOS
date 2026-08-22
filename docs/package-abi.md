# GhostOS user-space package ABI

This is the contract for signed packages that install and run Ring 3
programs. The Rust implementation is in [`ghostos-pkg`](../crates/pkg/src/lib.rs).
The compiler emits this format; the package daemon verifies it; the application
supervisor checks its metadata before the ELF loader maps the image.

## Version policy

| Contract | Current | Accepted | Behavior |
| --- | ---: | ---: | --- |
| Package API | `1.0` | `1.0` only | Reject an unknown or unsupported package contract before installation. |
| Inner package bundle | `SYNBNDL1`, version `1` | `1` only | Signed content-addressed payload and dependency metadata. |
| Application bundle | `SYNAPP01`, version `1` | `1` only | Signed application envelope around an inner package bundle. |
| Application manifest | schema `1` | `1` only | Fixed application identity, target, entry point, and resource limits. |

The public version constants are `PACKAGE_ABI_VERSION`,
`PACKAGE_BUNDLE_VERSION`, `APPLICATION_BUNDLE_VERSION`, and
`APPLICATION_MANIFEST_SCHEMA` in `ghostos-pkg`. The application parser uses the
same manifest schema constant. There is no package migration path yet. A
future incompatible format gets a new version and a new writer; old bytes are
not edited in place.

## Inner package bundle: `SYNBNDL1`

All integers are big-endian. The fixed header is 144 bytes. Dependency content
IDs follow the fixed header, so the complete header is
`144 + dependency_count * 32` bytes. The payload follows the header.

| Offset | Size | Field |
| ---: | ---: | --- |
| `0` | 8 | magic `SYNBNDL1` |
| `8` | 2 | bundle version, `1` |
| `10` | 2 | header length |
| `12` | 8 | payload length |
| `20` | 8 | ELF entry offset within the payload |
| `28` | 1 | dependency count, at most `MAX_DEPENDENCIES` |
| `29` | 3 | reserved, must be zero |
| `32` | 32 | package content ID |
| `64` | 32 | payload content ID |
| `96` | 16 | signing-key ID |
| `112` | 32 | HMAC-SHA256 signature |
| `144` | 32 each | dependency content IDs |

The package content ID covers the payload, entry offset, and dependency list.
The payload content ID covers the payload itself. The signature covers the
magic, version, lengths, entry offset, both content IDs, signing-key ID, and
dependency IDs. A decoder rejects bad lengths, reserved bytes, content IDs,
unknown keys, and invalid signatures.

## Application bundle: `SYNAPP01`

The outer header is 144 bytes, followed by the inner `SYNBNDL1` bytes and a
160-byte application metadata record.

| Offset | Size | Field |
| ---: | ---: | --- |
| `0` | 8 | magic `SYNAPP01` |
| `8` | 2 | application bundle version, `1` |
| `10` | 2 | outer header length, `144` |
| `12` | 8 | inner bundle length |
| `20` | 8 | metadata length, `160` |
| `28` | 32 | inner package content ID |
| `60` | 32 | metadata content ID |
| `92` | 16 | signing-key ID |
| `108` | 32 | HMAC-SHA256 signature |
| `140` | 4 | reserved, must be zero |

The outer signature covers the inner bundle hash and metadata hash. Both the
outer signature and the inner package signature must verify with a trusted key.
The inner and outer package IDs must match.

### Application metadata

Metadata integers are big-endian. Unused bytes must be zero.

| Offset | Size | Field |
| ---: | ---: | --- |
| `0` | 2 | application manifest schema, `1` |
| `2` | 1 | target: `1` x86_64, `2` AArch64 |
| `3` | 1 | kind: `1` service, `2` interactive, `3` batch |
| `4` | 1 | UTF-8 name length, `1..48` |
| `5` | 1 | reserved, zero |
| `6` | 8 | ELF entry offset |
| `14` | 8 | memory limit in bytes |
| `22` | 8 | CPU limit in microseconds; zero means no explicit limit |
| `30` | 8 | heap limit in bytes |
| `38` | 32 | debug-symbol content ID, or zero |
| `70` | 32 | compiler build-record content ID |
| `102` | 48 | UTF-8 application name, zero-padded |
| `150` | 10 | reserved, zero |

The package daemon stores metadata only after verifying the complete signed
bundle. Launch additionally requires the manifest supplied by the caller to
match the signed target, entry offset, and resource limits. Dependencies must
already be installed, and the supervisor still applies capability policy and
process limits. The package ABI never grants capabilities by itself.

## Compatibility and release rules

1. Keep `SYNBNDL1` and `SYNAPP01` field offsets and meanings unchanged within
   version `1`.
2. Never reuse a removed version or reinterpret an old field.
3. Reject unknown versions before package installation or activation.
4. Rebuild and sign packages for a different target or older release; do not
   patch bundle bytes or metadata in place.
5. A release changing this ABI updates this page, the
   [compatibility matrix](compatibility-matrix.md), the decoder and writer,
   and compatibility evidence.

The normal producer is `cargo ghostos package`. The normal consumer is
`PackageDaemon::install_application_bundle`, followed by the application
supervisor's signed metadata checks.
