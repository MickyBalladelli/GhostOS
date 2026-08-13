# Bounded workflow soaks

Run all bounded workflow soaks with:

```sh
SYNOS_SOAK_RUNS=3 ./scripts/soak.sh
```

The runner covers boot, shell, filesystem, network, compiler, cluster, VM,
lifecycle, and capability campaigns. The lifecycle campaign repeats reboot,
snapshot-based suspend/resume, memory hotplug, and init-service restart paths.
The capability campaign repeatedly creates, delegates, revokes, deletes, and
reuses capability slots while rejecting stale generations. It accounts for
pages, handles, IRQ routes, timers, capabilities, and worker tasks, and fails
if any ownership remains after a cycle.
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

Run only the lifecycle campaign, with a longer cycle count:

```sh
SYNOS_SOAK_RUNS=3 SYNOS_LIFECYCLE_CYCLES=512 ./scripts/soak.sh --scenario lifecycle
```

Run only the capability campaign, with a longer cycle count:

```sh
SYNOS_SOAK_RUNS=3 SYNOS_CAPABILITY_SOAK_CYCLES=4096 ./scripts/soak.sh --scenario capabilities
```

Use `SYNOS_SOAK_<WORKFLOW>_COMMAND` to replace a command, for example
`SYNOS_SOAK_VM_COMMAND`. The report remains machine-readable and includes the
resolved command, run count, timeout, tolerance, result state, findings, and
resource deltas. Lifecycle runs also retain one detailed ownership report per
run at `build/soak/lifecycle/run-<n>.lifecycle.json`.
