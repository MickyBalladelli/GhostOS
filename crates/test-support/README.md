# synos-test-support

Shared test-only helpers for SynOS crates and integration tests.

The crate provides deterministic boot, memory-map, capability, identity,
node, clock, entropy, packet, disk, SynFS-volume, manifest, wire-frame, and
terminal fixtures. It also provides bounded in-memory block, network, IPC,
storage, attestation, and accelerator doubles; one-shot failure injection;
LIFO cleanup guards; and checked-in golden fixtures.

Use it from a test target or a crate's `dev-dependencies`:

```toml
[dev-dependencies]
synos-test-support = { path = "../test-support" }
```

Do not add it to production dependencies. The doubles model test behavior and
are not hardware drivers, cryptographic implementations, or production IPC.
Every fault is consumed once, so a test can assert exactly which operation
failed. `TestScope` combines a fixed seed with a cleanup guard; its `Drop`
implementation performs best-effort cleanup after a panic.

Property tests use `property::run`. The default seed and case count are stable,
and a failure prints `SYNOS_PROPERTY_SEED` plus `SYNOS_PROPERTY_CASE` for exact
replay. Set `SYNOS_PROPERTY_CASES` to bound local runs. The bounded queue,
capability, and lease models are reference behavior for state-machine tests;
they are not production implementations.
