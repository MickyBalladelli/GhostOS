# ghostos-http

Heap-free HTTP/1 and gRPC primitives for GhostOS Ring 3 services.

`Router` provides fixed-capacity method/path dispatch with per-route capability
rights. `GrpcRouter` validates gRPC requests and dispatches framed protobuf
messages. `HttpServer` is a bounded polling state machine over the shared IPC
rings and owner-bound socket capabilities supplied by `ghostos-netd`.

Applications provide request, response, and handler scratch buffers. The crate
does not allocate, block, open raw sockets, or bypass the network daemon.
