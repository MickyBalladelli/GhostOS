# Continuous profiling

SynOS records bounded folded-stack samples through
[`synos-observability`](../crates/observability/src/profiling.rs). The sampler
is overwrite-oldest and lock-free, so a slow collector cannot block boot, IPC,
the scheduler, SynFS, networking, package activation, compiler builds, VM
execution, or client RPC.

Each producer records stable generated symbol IDs, not instruction addresses.
The archive carries the source revision, a SHA-256-derived redacted host ID,
sampling period, timestamps, sample counts, and dropped-sample counts. It has
a versioned binary format and checksum. No host name, address, path, argument,
payload, or identity is accepted by the archive.

The nine domains are wired at their central paths:

| Domain | Sampling point |
| --- | --- |
| boot | `kernel_entry` |
| IPC | successful kernel send |
| scheduler | successful context dispatch |
| SynFS | filesystem daemon dispatch |
| networking | network service execution |
| package activation | application supervisor launch |
| compiler builds | compiler host-run entry |
| VM execution | enabled VM profile hook |
| client RPC | SDK transport call |

Collect and symbolize an archive with a generated symbol map:

```sh
python3 scripts/profile.py symbolize \
  --input build/profiles/latest/profile.bin \
  --symbols build/profiles/latest/symbols.json \
  --output build/profiles/latest/profile.json
python3 scripts/profile.py retain \
  --input build/profiles/latest/profile.json \
  --root build/profiles/retained
```

Retention is keyed by `revision/host_id.json`. Two hosts running the same
revision can be compared after symbolization:

```sh
python3 scripts/profile.py compare \
  --left build/profiles/retained/<revision>/<host-a>.json \
  --right build/profiles/retained/<revision>/<host-b>.json
```

The Rust regression coverage checks bounded overflow, symbol-only samples,
redacted host identity, deterministic aggregation, revision retention, and
export checksum validation.
