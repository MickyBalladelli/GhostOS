# ghostos-inference

High-level, heap-free AI execution for GhostOS.

- `gateway` decodes OpenAI `/v1/models`, `/v1/completions`, and
  `/v1/chat/completions` bodies, produces JSON/SSE responses, and supports a
  framed protobuf gRPC service boundary described by
  `proto/ghostos_inference.proto`.
- `service` registers cluster-backed models and coordinates RAM/VRAM KV-cache
  reservations with dual-journal token checkpoints and failover.
- `agent_state` periodically stores checksummed agent images in immutable GhostFS
  versions and pins the latest CoW root for recovery.

The gateway does not own a socket. A Ring 3 transport feeds request frames into
`InferenceGateway`, so HTTP and gRPC transports share one execution path.
