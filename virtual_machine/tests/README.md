# VM test tiers

The VM tests use stable names from `inventory.toml`. Source modules, public API
symbols, and device implementations are written to `generated-inventory.toml`.
Regenerate that file after changing the public surface:

```sh
python3 scripts/generate-vm-inventory.py
```

The generator preserves assignments for existing symbols. New public API and
device entries have an empty `tests` list; add a stable test ID to that list
before committing.

The IDs point to executed records under
`build/test-evidence/<run-id>/<tier>/<test-id>/evidence.json`. Each tier also
has a `result.json` with `passed`, `failed`, or `skipped` plus a reason.
`test-all.sh` records fast deterministic VM evidence. `full-validation.sh`
keeps QEMU, fuzz, and soak evidence in separate tier directories, including
the boot-image SHA-256 digest where applicable.

- `foundation_59_1.rs`: fast deterministic fixtures, fake devices, faults,
  cleanup, and golden-file checks.
- `matrix_59_11.rs`: deterministic VM integration matrix.
- `cluster.rs`: deterministic multi-node, network-fault, CXL, shared-memory,
  heartbeat, coherence, fencing, rejoin, and workload-failure fixtures. The
  harness is bounded and exposes deterministic evidence snapshots; QEMU cluster
  validation remains opt-in.
- `main.rs` unit tests: CLI parsing and command behavior.
- `qemu_matrix_59_11.rs` and `test_environments.rs`: opt-in QEMU and storage
  integrity checks.
- `soak_leaks.rs`: repeated translation-cache and terminal teardown checks;
  `lifecycle_soak.rs` repeats reboot, suspend/resume, memory hotplug, and
  service restart while checking pages, handles, IRQ routes, timers,
  capabilities, and worker tasks;
  `scripts/vm-soak.py` adds focused VM host snapshots, while
  `scripts/soak.py` runs the bounded VM workflow alongside boot, shell,
  filesystem, network, compiler, cluster, and lifecycle workflows.
- `benches/bounded.rs`: bounded decode, translation, memory, interrupt,
  storage, network, and terminal performance data in JSON Lines format.
- `snapshot_terminal_network_10_5.rs`: deterministic snapshot, terminal,
  loopback, and storage/network matrix checks.
- `../fuzz/fuzz_targets/vm_*.rs`: decoder, device, disk-image, snapshot,
  terminal, and migration fuzz boundaries with retained replayable inputs.
- cluster and hardware-accelerated runs are opt-in external tiers named in the
  inventory; they reuse the same cleanup and evidence contract.
- `quality_gates.rs`: deterministic checks for the inventory contract and the
  six required device-boundary scenarios.

Run `python3 scripts/validate-vm-quality.py` from the repository root to check
source-module/API inventory, device scenarios, serial boot paths, fuzz targets,
mutation wiring, and local inventory enforcement.

Do not put wall-clock, host-random, or host-path assumptions in the fast tier.
