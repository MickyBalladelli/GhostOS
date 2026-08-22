# ghostos-client-sdk

Portable, heap-free GhostOS RPC client and frontend gateway protocol. The core is
`no_std` and contains no platform networking, async runtime, or allocator.

## Compatibility policy

The stable user-space SDK contract is `SDK_API_VERSION` `1.0`. Rust and Swift
SDKs share this source-level contract; their package or toolchain versions do
not change the wire contract. The supported range is exposed by
`SDK_MINIMUM_API_VERSION` and `SDK_MAXIMUM_API_VERSION`.

Follow [`docs/sdk-compatibility.md`](../../docs/sdk-compatibility.md) when
publishing an SDK or changing a method, schema, status, frame limit, or
transport rule. Compatibility is checked before decoding, and unsupported
versions return stable compatibility errors.

## Transport boundary

Implement `RpcTransport::round_trip` with the host platform:

- macOS and iOS: `URLSession`
- Android: the app HTTP client or a JNI-owned bridge
- WebAssembly: browser `fetch`
- native Rust: the selected TLS/HTTP stack

The transport sends one complete request frame and returns one complete response
frame. HTTP gateways use `POST` with
`Content-Type: application/vnd.ghostos.rpc`.

## Wire contract

All integers are big-endian. Every frame begins with this 24-byte header:

| Offset | Bytes | Value |
| --- | ---: | --- |
| 0 | 4 | `SYRP` |
| 4 | 1 | protocol version (`1`) |
| 5 | 1 | method |
| 6 | 2 | flags |
| 8 | 8 | request ID |
| 16 | 4 | payload byte count |
| 20 | 2 | response status, zero in requests |
| 22 | 2 | reserved |

Flag bit zero means that a 192-byte `SYCA` cryptographic capability starts the
request payload. Responses never echo authority. `FrameHeader` rejects unknown
versions, flags, methods, status codes, oversized payloads, and truncated
frames.

The SDK currently exposes:

- `Client::cluster_state`
- cluster summary, membership, invitation, join/leave-plan, health, resource,
  and audit schemas through bounded RPC methods
- lifecycle methods for create, join, leave, and remove
- `Client::subscribe` and bounded `Client::poll` for membership, health,
  topology, resource, and lifecycle changes
- `Client::submit_job`
- `Client::delegate_capability`
- `FrontendGateway::handle`

`GatewayService` owns authentication and authorization policy. The dispatcher
only validates framing and typed payloads; possession of a decoded capability
does not itself authorize an operation.

HTTP gateways use `POST /...` with `application/vnd.ghostos.rpc`. The
`ghostos-http` crate provides the content-type and response envelope helpers;
its `GrpcRouter` provides the equivalent bounded gRPC dispatch path.
