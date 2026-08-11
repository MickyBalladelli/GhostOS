# Compiler workflow bounds

The compiler service keeps successful package and payload identities in one
bounded immutable store. Local cache hits require that store. Remote cache
records require an exact content key, target match, non-zero artifact IDs, and
a proof over the key and result. Offline requests never call a remote cache.

Registry dependencies are resolved into deterministic parallel waves. A cycle
is rejected before the job enters the build queue. Completion checks cache
capacity, cancellation state, target, and artifact identity before publication.
Cancelled, failed, expired, and crashed jobs publish no result or cache entry.

Measure the four required paths on the same host:

```sh
python3 scripts/benchmark-compiler-workflows.py \
  --manifest-path service/Cargo.toml --bin service --release \
  --samples 5 --output build/benchmarks/compiler-workflows.json
```

The report records clean, warm, offline, and cancelled wall time with p50/p95
and max bounds, plus host and command metadata. Temporary target roots are
isolated per sample and removed after the run.

The fixed service ceilings are 16 jobs, 32 local content-cache records, 64
shared immutable artifacts, 16 dependency nodes, and a five-second cancellation
grace period.
