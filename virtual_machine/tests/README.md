# VM test tiers

The VM tests use stable names from `inventory.toml`.

- `foundation_59_1.rs`: fast deterministic fixtures, fake devices, faults,
  cleanup, and golden-file checks.
- `matrix_59_11.rs`: deterministic VM integration matrix.
- `cluster.rs`: deterministic multi-node, network-fault, CXL, shared-memory,
  heartbeat, coherence, fencing, rejoin, and workload-failure fixtures. The
  harness is bounded and exposes deterministic evidence snapshots; QEMU cluster
  validation remains opt-in.
- `main.rs` unit tests: CLI parsing and command behavior.
- `qemu_matrix_59_11.rs` and `test_environments.rs`: opt-in QEMU and bounded
  performance checks.
- `snapshot_terminal_network_10_5.rs`: deterministic snapshot, terminal,
  loopback, and storage/network matrix checks.
- cluster and hardware-accelerated runs are opt-in external tiers named in the
  inventory; they reuse the same cleanup and evidence contract.

Do not put wall-clock, host-random, or host-path assumptions in the fast tier.
