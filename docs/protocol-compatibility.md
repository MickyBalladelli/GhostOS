# Protocol compatibility contract

All six network-facing boundaries expose a checked entry point backed by
`ghostos-protocol::ProtocolGuard` before a message reaches the existing parser or
handler. The guard has no socket or allocator dependency.

| Traffic | Max message | Inflight bytes | Replay window | Auth failures | First reconnect delay |
| --- | ---: | ---: | ---: | ---: | ---: |
| HTTP | 64 KiB | 64 KiB | 64 | 3 | 250 ms |
| gRPC | 1 MiB | 1 MiB | 64 | 3 | 1 s |
| SDK | 4 KiB | 4 KiB | 64 | 3 | 16 ms |
| Remote terminal | 16 KiB | 16 KiB | 64 | 3 | 64 ms |
| Mesh | 4 KiB | 64 KiB | 64 | 3 | 64 ms |
| Cluster | 4 KiB | 64 KiB | 64 | 3 | 64 ms |

The local and peer version ranges are negotiated by selecting the highest
version in their intersection. No intersection rejects the connection before
message processing. A message is accepted only after negotiation and only if
its byte length is within the traffic cap.

Sequences start at one. The highest accepted sequence and the previous 63
positions form a sliding replay window. A duplicate or sequence older than the
window is rejected. Three consecutive authentication failures lock the guard;
a successful authentication clears the failure counter while the guard is not
locked.

Outbound work reserves both one message slot and its bytes. Saturation returns
`Backpressure`; it never allocates or silently drops data. Disconnects schedule
up to eight retries with exponential backoff. Successful reconnect resets the
attempt counter. Exhaustion returns `ReconnectExhausted` and requires operator
or service intervention.

The direct matrix regression is in `crates/protocol/src/lib.rs` and exercises
each traffic class through negotiation, size, replay, authentication,
backpressure, and reconnect paths. HTTP and gRPC use checked frame parsers;
the SDK uses `decode_frame_checked`; remote terminal, mesh, and cluster expose
checked message admission helpers.
