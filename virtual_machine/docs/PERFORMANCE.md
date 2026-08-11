# Performance tuning guide

## Bounded benchmark suite

Run the repeatable VM harness from the repository root:

```bash
python3 scripts/benchmark.py \
  --output build/benchmarks/latest/report.json \
  -- cargo bench -p synos-vm --bench bounded
```

The harness runs two warmups and nine measured samples by default. It records
fixed hardware metadata and a host signature, then emits p50/p95/p99 latency,
throughput, confidence intervals, allocation counts/bytes, and optional CPU
cycle and energy readings. A noisy workload is `inconclusive`; it is never
silently treated as a pass.

Use a report from the same hardware signature to enforce regression budgets:

```bash
python3 scripts/benchmark.py \
  --baseline build/benchmarks/baseline.json \
  --output build/benchmarks/latest/report.json \
  -- cargo bench -p synos-vm --bench bounded
```

Create a baseline only from a passing run:

```bash
python3 scripts/benchmark.py \
  --write-baseline build/benchmarks/baseline.json \
  -- cargo bench -p synos-vm --bench bounded
```

Budgets live in [`benchmarks/budgets.toml`](../../benchmarks/budgets.toml).
Relative budgets fail on latency, throughput, or allocations. Absolute
ceilings apply even before a baseline exists.

The underlying bounded benchmark emits one JSON Lines metadata record, followed
by one result record for each workload. It has fixed work limits:

| Workload | Fixed work |
| --- | --- |
| Decode | 200,000 complex instruction decodes |
| Translation | 20,000 cold 16-instruction blocks |
| Memory | Four write/read passes over 4 MiB |
| Interrupt | 200,000 APIC signal/accept/EOI cycles |
| Storage | Write and read 256 RAW sectors |
| Network | 100,000 loopback Ethernet deliveries |
| Terminal | 10,000 translations of a 4 KiB input block |

Each result includes `work_units`, `elapsed_ns`, `rate_per_second`, and a
deterministic `checksum`. The checksum and work count make workload drift
visible; elapsed time remains machine-dependent. Metadata records the crate
version, supplied revision, host OS/architecture, logical CPU count, and build
profile. Setup and temporary-file creation happen outside measured intervals.
The storage image is cleaned up after the bounded run.

The VM is an interpreter with a translated-block cache. Tune the execution
engine through the library API:

```rust
use synos_vm::{Vm, VmConfig};

let mut vm = Vm::with_config(VmConfig {
    memory_size: 256 * 1024 * 1024,
    ..VmConfig::default()
});

let engine = vm.execution_mut();
engine.config_mut().max_block_instructions = 64;
engine.config_mut().hot_threshold = 256;
engine.config_mut().cache_capacity = 8_192;
engine.config_mut().enable_jit = true;
engine.config_mut().enable_profiling = false;
```

## Knobs

| Setting | Increase it when | Cost |
| --- | --- | --- |
| `max_block_instructions` | Guest has long straight-line code | Larger translation work and block memory |
| `hot_threshold` | Loops run long enough to benefit from promotion | Hot code waits longer before promotion |
| `cache_capacity` | The workload has many code pages or mode changes | More host memory |
| `enable_jit` | Repeat loops dominate runtime | Less useful for short bring-up runs |
| `enable_profiling` | You need counters or a profile hook | Counter and callback overhead |

Start with the defaults. For a short boot smoke test, disable profiling and
use `run_for_steps`. For a long-running guest, keep the cache enabled and
increase capacity only after observing cache evictions in
`vm.execution().stats()`.

## Measure the right thing

The counters expose instructions, translated blocks, cache hits and misses,
compiled blocks, and evictions. Record them after a stable guest phase, not
while the BIOS is still changing code and paging state. Compare the same
kernel, RAM size, step budget, and host build each time.

Device work can dominate a storage or network workload. Those devices perform
DMA after each CPU dispatch, so very small dispatches add host overhead. Do
not trade away device boundaries or interrupt delivery to gain a benchmark
number; guest-visible timing and correctness depend on them.

Snapshots clear the translated cache on restore. The first run after restore
therefore includes translation warm-up.
