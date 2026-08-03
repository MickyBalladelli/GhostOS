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
