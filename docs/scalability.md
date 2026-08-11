# Scheduler and service scalability

The five qualified CPU tiers are `1`, `2`, `8`, `32`, and `128`. The shared
policy in [`crates/observability/src/scaling.rs`](../crates/observability/src/scaling.rs)
builds a two-word 128-bit online mask, reserves the last one-eighth of CPUs
for isolated workloads at tiers of eight CPUs and above,
and gives scheduler, IPC, timer, logging, and audit work the housekeeping mask.
This prevents service work from entering an isolated CPU by accident.

The kernel uses the same policy through `Scheduler::configure_scale_policy`.
`CpuMask` is two words, so CPU 127 is representable. Scheduler dispatch,
timer ticks, and IPC sends already reject isolated CPUs through
`CorePartition`. `LogDaemon::poll_on_cpu` and `AuditDaemon::poll_on_cpu` apply
the same admission rule to Ring 3 service workers. Existing bounded batching
and fixed-capacity rings remain the queue and fairness boundaries.

## Evidence command

Run on each host with the same release profile and workload size:

```sh
SYNOS_SCALE_ITERATIONS=20000 python3 scripts/benchmark.py \
  --warmups 2 --samples 9 --no-optional-metrics \
  --output build/benchmarks/scalability.json -- \
  cargo bench -p synos-kernel --bench scalability -- --nocapture
```

The JSONL records contain one result for every path and CPU tier. Each record
includes measured p50/p99 elapsed time and throughput, the effective
housekeeping and isolated CPU counts, the bounded queue saturation point, and
the declared shared-lock saturation point. `queue_saturation` must equal the
fixed queue capacity; `p99_ns` must remain below the path budget after unit
conversion:

| path | p99 budget | queue capacity | lock saturation / CPU |
| --- | ---: | ---: | ---: |
| scheduler | 100 us | 256 | 4,096 ops |
| IPC | 250 us | 1,024 | 8,192 ops |
| timers | 100 us | 256 | 4,096 ops |
| logging | 5 ms | 256 | 2,048 ops |
| audit | 10 ms | 256 | 1,024 ops |

The benchmark does not invent a 128-CPU result when the host has fewer CPUs:
the policy and bounded-path run still qualify the layout, while host metadata
from `scripts/benchmark.py` identifies whether the measurement was native or
an emulated tier run. Retain the resulting JSON with the source revision,
command, host signature, and configuration next to release evidence.

Smoke evidence recorded 2026-08-11 at revision `bbb8b3b93e5e02dc63fc28155bb923e5400daada`.
Command: `SYNOS_SCALE_ITERATIONS=100 cargo bench -p synos-kernel --bench scalability -- --nocapture`.
It used the optimized bench profile on Darwin 25.5.0 arm64. All five paths
stayed within their p99 budgets at all five policy tiers; the observed p99
operation latency maximum was 983 ns, minimum reported throughput was
1,072,869 operations/s, and queue saturation matched the declared capacities.
The 8/32/128 rows were policy-tier runs on this host, not a claim of 128
physical host CPUs.
