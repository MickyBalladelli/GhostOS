# synos-client-sdk

Portable, heap-free SynOS RPC client and frontend gateway protocol. The core is
`no_std` and contains no platform networking, async runtime, or allocator.

## Transport boundary

Implement `RpcTransport::round_trip` with the host platform:

- macOS and iOS: `URLSession`
- Android: the app HTTP client or a JNI-owned bridge
- WebAssembly: browser `fetch`
- native Rust: the selected TLS/HTTP stack

The transport sends one complete request frame and returns one complete response
frame. HTTP gateways use `POST` with
`Content-Type: application/vnd.synos.rpc`.

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

HTTP gateways use `POST /...` with `application/vnd.synos.rpc`. The
`synos-http` crate provides the content-type and response envelope helpers;
its `GrpcRouter` provides the equivalent bounded gRPC dispatch path.
