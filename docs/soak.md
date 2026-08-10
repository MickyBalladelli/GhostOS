# Bounded workflow soaks

Run all bounded workflow soaks with:

```sh
SYNOS_SOAK_RUNS=3 ./scripts/soak.sh
```

The runner covers boot, shell, filesystem, network, compiler, cluster, and VM.
Each command has a per-run timeout. It writes one JSON report to
`build/soak/report.json` and keeps stdout and stderr beside the scenario that
produced them.

Each run records before/after snapshots and deltas for scratch files, bytes,
sockets, locks, child processes, file descriptors, runner RSS, child CPU time,
and terminal state. A run fails on a non-zero command, timeout, leftover
scratch resource, new child process, terminal mutation, or runner RSS growth
above `SYNOS_SOAK_MEMORY_TOLERANCE_BYTES`. Failed runs keep their scratch path
for inspection; passing runs remove it.

Run one workflow when debugging:

```sh
SYNOS_SOAK_RUNS=3 ./scripts/soak.sh --scenario filesystem
```

Use `SYNOS_SOAK_<WORKFLOW>_COMMAND` to replace a command, for example
`SYNOS_SOAK_VM_COMMAND`. The report remains machine-readable and includes the
resolved command, run count, timeout, tolerance, result state, findings, and
resource deltas.
