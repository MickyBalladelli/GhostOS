# Release claims

Release notes may use `scalable`, `durable`, `secure`, or `real-time` only when
the release has a matching record in `release-claims.json`. The record must
name the workload, give a measured threshold and observed value, identify the
host configuration, and point to a retained evidence file by SHA-256 digest.

The manifest is revision-bound and checked before the release gate and VM
archive are produced:

```sh
python3 scripts/validate-release-claims.py \
  --claims build/release/release-claims.json \
  --evidence-dir build/test-evidence/<run-id>
```

Minimal record shape:

```json
{
  "schema": 1,
  "kind": "ghostos-release-claims",
  "revision": "<git revision>",
  "claims": [
    {
      "id": "scheduler-scale",
      "term": "scalable",
      "statement": "The scheduler is scalable for the bounded tier workload.",
      "workload": "scheduler dispatch at CPU tiers 1, 2, 8, 32, and 128",
      "measured_threshold": {
        "metric": "operation p99 latency",
        "operator": "<=",
        "value": 100000,
        "observed": 983,
        "unit": "ns"
      },
      "host_configuration": {
        "system": "Darwin",
        "release": "25.5.0",
        "architecture": "arm64",
        "configuration": "release profile; 20,000 iterations; 2 warmups; 9 samples"
      },
      "artifact": {
        "path": "benchmarks/scalability.json",
        "sha256": "<sha256 of retained evidence file>",
        "description": "retained scalability benchmark output"
      }
    }
  ]
}
```

The artifact path is relative to the evidence directory, cannot escape it or
be a symlink, and must retain the exact digest recorded in the manifest.
