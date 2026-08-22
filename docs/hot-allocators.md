# Hot object allocators

`ghostos_kernel::HotObjectAllocator` provides four bounded pools: IPC, packet,
timer, and scheduler objects. Each NUMA node owns one bitmap pool per CPU and
object kind. The allocator checks the current CPU first, then same-node CPUs,
then at most the configured number of remote nodes. No heap or unbounded scan
is used.

The returned `HotAllocation` is an ownership token. Reclamation uses the token
to return the slot to its original CPU and node, and `reclaim_on` records when a
different CPU performs the reclamation. Double reclaim and forged node or slot
metadata are rejected.

`report(kind)` exposes the mixed-workload evidence surface:

- locality counts for current-CPU, same-node, and remote-node allocations;
- capacity, live objects, free space, largest free runs, and per-mille
  fragmentation;
- probe histograms, average probe steps, and the maximum probe depth as a
  bounded tail-latency proxy;
- successful, remote, and invalid reclamation counts.

Callers size the pools for the hot-path object class and pass the CPU-to-NUMA
mapping discovered during platform setup. A zero remote fallback limit gives a
strict local policy; a positive limit is still bounded by the configured node
count. The repeatable mixed workload probe is
[`kernel/benches/hot_allocator.rs`](../kernel/benches/hot_allocator.rs); it
reports p50/p95/p99 operation latency beside the allocator counters.
