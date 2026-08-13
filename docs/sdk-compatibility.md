# User-space SDK compatibility policy

This is the stable compatibility policy for SynOS user-space SDKs. It applies
to the Rust `synos-client-sdk` crate, the Swift `SynOSClient` package, and any
future SDK that implements the same contract.

## Stable contract

The SDK source-level contract is version `1.0`. The authoritative registry is
[`synos-api-compat::SDK_API`](../crates/api-compat/src/lib.rs). The Rust SDK
exports the contract and its accepted range as `SDK_API_VERSION`,
`SDK_MINIMUM_API_VERSION`, and `SDK_MAXIMUM_API_VERSION`. Swift exposes the
same value as `SynOSCompatibility.sdkApiVersion`.

SDK package versions and toolchain versions are implementation versions. They
must not be used as a substitute for the SDK contract or the wire version.

## Compatibility layers

| Layer | Current contract | Promise |
| --- | --- | --- |
| SDK source API | `1.0` | Additive changes are minor-version work; breaking type, method, or semantic changes require a new major SDK API. |
| `SYRP` client RPC | protocol `1` | Keep the 24-byte header, byte order, frame limit, method IDs, and status meanings stable. New methods get new IDs. |
| Shared transport | SDK traffic class `1` | Negotiate the highest common version before processing messages; retain the bounded size, replay, authentication, backpressure, and reconnect rules. |
| Native user ABI | schema `1`, revision `4` | User programs use the versioned syscall and package contracts; a revision is not permission to reinterpret the schema. |
| Public errors | `SYNOS-COMPAT-*` and SDK status mappings | Error codes and retry meaning remain machine-readable and stable across Rust and Swift. |

The SDK may be updated independently of the operating system implementation as
long as these layers remain within their accepted ranges. A successful compile
alone is never compatibility evidence.

## Version rules

- Patch releases fix implementation defects and documentation. They do not
  change public meanings, encoded bytes, limits, or status mappings.
- Minor SDK releases may add types, methods, and capabilities. Existing
  methods, method IDs, field meanings, and error behavior remain valid.
- A major SDK release is required for a breaking source, wire, or semantic
  change. It must publish an adapter or migration plan when old clients can be
  supported safely.
- Removed method IDs are never reused. An unsupported method returns the
  existing stable protocol error; it is not silently reinterpreted.
- Deprecated APIs remain available through the next compatible minor release
  and are removed only in a major release. Deprecation must name the
  replacement and appear in the changelog and SDK documentation.
- An SDK must reject an unsupported version before decoding request payloads or
  invoking a service. It must report the stable compatibility code and the
  accepted range.

## Release requirements

Every SDK contract change must update, in this order:

1. the decoder, encoder, or typed API;
2. compatibility regressions for current, deprecated, and rejected versions;
3. the Rust and Swift bindings, if the public surface changes;
4. [`api-versioning.md`](api-versioning.md) and the
   [`compatibility-matrix.md`](compatibility-matrix.md);
5. release evidence identifying the source revision and negotiated versions.

The release must state whether old SDKs can talk to the new service, whether a
new SDK can talk to the old service, and what migration is required when either
direction is unsupported. Do not edit encoded data or reinterpret an old
method in place.

## Current support

The current stable promise is:

| Client | SDK API | Wire | Platforms |
| --- | ---: | ---: | --- |
| Rust `synos-client-sdk` | `1.0` | `SYRP` v1 | `no_std`, native Rust and WebAssembly transports supplied by the host |
| Swift `SynOSClient` | `1.0` | `SYRP` v1 | macOS 14+ and iOS 17+ |

The accepted range is intentionally `1.0..=1.0` until a compatible minor
contract is implemented and evidence is recorded. The policy itself does not
claim support for POSIX compatibility, private service IPC, or undocumented
internal Rust types.
