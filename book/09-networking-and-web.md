# 9. Networking, HTTP, and Remote Surfaces

GhostOS networking is asynchronous, capability-controlled, and designed around buffer ownership.

## `ghostos-netd`

`ghostos-netd` is a heap-free Ring 3 TCP/IP service built on `smoltcp`. NIC drivers loan fixed packet slots to the stack. Ingress and egress frames stay in their original buffers while ownership changes.

Applications submit socket operations through shared IPC rings. A socket token is:

- generation-checked;
- owner-bound;
- associated with an authenticated principal;
- authorized separately for listen, connect, send, receive, inspect, and close.

## Packet path

```text
NIC driver -> packet slot -> netd -> socket capability -> application buffer
      ^                                               |
      +------------- completion / ownership ----------+
```

The stack polls with an ingress budget. That keeps a flood from taking all scheduler time.

## HTTP and gRPC

`ghostos-http` provides heap-free HTTP/1 parsing, response encoding, fixed-capacity method/path routing, and gRPC framing. Routes carry web-service rights. The authenticated principal and attenuated grant are checked before handler dispatch.

The server does not own a raw network backend. It submits open, listen, receive, send, and close operations through `ghostos-netd` IPC rings.

## Client SDK and remote services

`ghostos-client-sdk` is `no_std` and allocation-free. Its versioned `SYRP` frames carry bounded typed RPCs and optional cryptographic capabilities for cluster snapshots, job submission, capability delegation, membership, invitations, plans, health, resources, topology, and audit activity.

`FrontendGateway` decodes and bounds-checks a request before it reaches a policy-owning service. Subscriptions use cursors and fixed poll batches, so a slow dashboard cannot grow server state without bound. The transport trait lets native HTTP and browser `fetch` share the same SDK contract.

The repository also contains an Apple Swift client and SwiftUI control surface for macOS and iOS. It displays node health and resources, submits bounded jobs, and requests restricted grants through a TLS HTTP gateway.

`ghostos-webterm` exposes remote terminal workflows. `ghostos-remote-display` carries remote console/display data. Both preserve authentication, framing, resize, reconnect, and cleanup contracts.

The web terminal models VT100/VT420/DECterm behavior: UTF-8, cursor movement,
scrolling regions, erase and insert operations, SGR colors, DEC private modes,
and OSC framing. Dirty rows become fixed WebGPU cell instances, so a browser
updates only changed GPU ranges. The SSH gate accepts public-key proofs,
requires a shell capability, and binds each generation-checked session to its
principal.

Remote display surfaces move through available, capturing, encoding, and
in-flight lease states. NV12, P010, RGBA, and BGRA planes stay in shared memory.
The AV1 encoder returns another shared descriptor, and the RFC 9364 RTP
packetizer creates scatter/gather views without copying the bitstream. The
display daemon negotiates AV1-only WebRTC sessions, validates expiring view and
input grants, and uses loss/latency/bitrate feedback plus keyframe requests.

## Firewall and policy

The network feature set includes capability-gated packet filtering, policy records, and audit evidence. A remote request must pass both network policy and the caller’s resource capability.

## Easy example: an HTTP request

```text
client capability
    -> open socket
    -> connect
    -> receive bounded request
    -> route method + path
    -> verify service right
    -> handler writes shared response buffer
    -> send + close
```

If the request is too large, the route is absent, the principal lacks rights, or the socket generation is stale, the system returns a structured error instead of improvising.
