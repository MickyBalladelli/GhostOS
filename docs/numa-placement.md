# NUMA placement

`ghostos-numa` owns the bounded topology and placement contract used by the
kernel scheduler, platform queues, storage workers, and network interrupt
polling. The topology is a fixed CPU-to-node table; placement never scans or
allocates without a bound.

Each owner retains a `NumaReport` with decision counts for local CPU, local
node, remote node, and UMA fallback placement. Memory owners also record
remote access count and exact remote bytes at the ownership boundary. The
kernel hot allocator retains its per-kind locality report and accepts exact
remote-byte accounting through `record_remote_memory`.

If firmware supplies no topology, every owner starts with
`NumaTopology::uma()`. Invalid node requests select a bounded valid node, and
UMA hosts never report remote traffic. This keeps boot and VM paths safe while
still exposing placement decisions when a real topology is configured.

Regression coverage lives in `crates/numa/src/lib.rs` and covers invalid-node
fallback plus the no-remote-bytes UMA contract.
