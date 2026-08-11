# Lock contention evidence

The hot quota path now has three independent fair ticket shards: IPC,
page-fault, and memory accounting. `usage()` reads an atomic counter without
waiting. Each shard reports its active owner token, acquisitions, contended
acquisitions, spin work, and a bounded duration histogram.

The kernel DLM reports active owners, queued acquisitions, promotions, release
and expiry counts, plus wait and hold-duration histograms. DLM promotion keeps
FIFO order for overlapping ranges by rejecting any candidate with an older
overlapping waiter. Balancerd uses `release_at` on timed migration and restore
paths so daemon-held lease duration is measured with daemon time.

Run the repeatable comparison on the same host:

```sh
SYNOS_LOCK_ITERATIONS=100000 SYNOS_LOCK_WORKERS=4 \
  python3 scripts/benchmark.py --warmups 1 --samples 5 \
  --no-optional-metrics --output build/benchmarks/lock-contention.json -- \
  cargo bench -p synos-kernel --bench lock_contention -- --nocapture
```

Evidence recorded on revision `21cef4bc1d55f85f3c5aa2ad57e87e64152613f2`,
optimized benchmark profile, 10 logical CPUs:

| workload | p50 elapsed | p95 elapsed | p99 elapsed | throughput | active owners |
| --- | ---: | ---: | ---: | ---: | ---: |
| one worker | 3.70 ms | 3.84 ms | 3.85 ms | 27.0M op/s | 0 |
| four workers | 122.72 ms | 138.13 ms | 138.77 ms | 3.26M op/s | 0 |

The five-sample harness passed its declared noise and absolute checks. The
one-worker run had zero contention. The four-worker run recorded about 397k
contended acquisitions and 5.0M–7.1M spins per sample, with no owner left
active. FIFO ticket service and oldest-waiter DLM promotion preserve fairness;
there is no fairness bypass path.
