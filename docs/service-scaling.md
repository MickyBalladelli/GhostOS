# Horizontal service scaling

`synos-service-scale` is the shared bounded control plane for HTTP, remote
terminal, package, compiler, storage, and observability services.

An instance joins with a monotonically increasing generation and becomes
eligible for traffic only after `ready`. `drain` rejects new sessions by
moving the instance out of the ready set. Existing sessions use
`prepare_handoff`, `accept_handoff`, and `commit_handoff`; preparation refuses
to proceed while a session has in-flight work. The old instance can enter
`restart` only after all sessions and requests have moved or completed. A
restarted instance must rejoin with a higher generation, so stale completions
cannot mutate current state.

Each request carries a stable request ID and nonzero effect key. Completed
effects are retained in a fixed-capacity receipt ledger. A retry with the same
request or effect key returns the original receipt, while a different effect
for the same request is rejected. This makes the handoff boundary safe for
mutating package, compiler, storage, and observability operations as well as
read-only HTTP and terminal work.

The service crates expose typed controllers: `HttpScale`,
`RemoteTerminalScale`, `PackageScale`, `CompilerScale`, `StorageScale`, and
`ObservabilityScale`. All capacities are compile-time bounded and no heap or
background coordinator is required.
