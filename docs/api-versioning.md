# SynOS API versioning

SynOS treats public Rust, Swift, user-space SDK, wire, shell, package, snapshot,
and configuration boundaries as versioned contracts. The shared policy lives in
[`synos-api-compat`](../crates/api-compat/src/lib.rs). Format readers still
validate their own magic, lengths, fields, checksums, signatures, and feature
bits after this policy check.

## Contract registry

| Boundary | Current | Accepted | Migration | Deprecated entry point |
| --- | ---: | ---: | --- | --- |
| Rust public API | 1.0 | 1.0 | Publish a new major contract and adapter | `LEGACY_SNAPSHOT_V1` is the test adapter pattern |
| Swift public API | 1 | 1 | Publish a new client adapter | `SynOSCompatibility.legacyVersion` |
| Wire API | 1.0 | 1.0 | Negotiate before decoding | Stable `SYNOS-COMPAT-*` errors |
| Shell API | 1.0 | 1.0 | Export commands and recreate state | Stable `SYNOS-COMPAT-*` errors |
| Package API | 1.0 | 1.0 | Rebuild and sign with the target writer; see [`package-abi.md`](package-abi.md) | Stable `SYNOS-COMPAT-*` errors |
| Snapshot API | 2.0 | 1.0–2.0 | `snapshot-v1-to-v2` | `LEGACY_SNAPSHOT_V1` |
| Configuration API | 1.0 | 1.0 | Re-render and sign configuration | Stable `SYNOS-COMPAT-*` errors |
| User-space SDK | 1.0 | 1.0 | Add a new SDK major and an explicit adapter | No legacy entry point; use `SDK_API` |

An offered version below the accepted minimum returns `SYNOS-COMPAT-001`.
An offered version above the accepted maximum returns `SYNOS-COMPAT-002`.
Malformed ranges return `SYNOS-COMPAT-003`; a supported version with no
declared conversion returns `SYNOS-COMPAT-004`. These codes are stable and
must be preserved across Rust, Swift, shell, and wire status mappings.

## Migration examples

Old snapshot clients may continue to send version 1. The receiver first calls
`SNAPSHOT_API.check(ApiVersion::V1)`, records warning
`SYNOS-COMPAT-DEP-001`, then applies the declared `snapshot-v1-to-v2` conversion
before normal snapshot validation:

```text
synos-vm snapshot convert --from 1 --to 2 input.vm output.vm
```

Package and configuration downgrade is export/import or re-render/re-sign. No
bytes are edited in place. A wire or shell client with an unsupported version
is refused before parsing or mutation and receives the stable code in its
error response.

## Change procedure

1. Add the new range and explicit migration to `synos-api-compat`.
2. Keep the old decoder and add a compatibility regression for every accepted
   version, including its warning or stable rejection code.
3. Add a `#[deprecated]` Rust entry point or `@available(..., deprecated)` Swift
   entry point for the old call shape.
4. Update the compatibility matrix and this guide with a copyable migration.
5. Record the exact command and result in release evidence.

The user-space SDK additionally follows the rules in
[`sdk-compatibility.md`](sdk-compatibility.md). A Rust crate or Swift package
version never overrides the negotiated wire version or the SDK contract.
