# 18. Cookbook: Memorable Workflows

These recipes are short paths through the system. They are meant to help a new developer remember the project.

## Recipe 1: boot the kernel

```sh
./scripts/build-bios-image.sh
```

Memory hook: **stage 1 loads stage 2, stage 2 loads the kernel, the kernel receives BootInfo.**

## Recipe 2: boot in the VM

```sh
cargo build --release -p synos-vm
target/release/synos-vm \
  --kernel build/bios/kernel.bin \
  --append "console=serial0" \
  --steps 100000
```

Memory hook: **kernel, arguments, steps.**

## Recipe 3: use the shell

```text
HELP
DIRECTORY /DATA
CREATE /DATA/hello
TYPE /DATA/hello
SHOW MEMORY
SHOW CLUSTER/HEALTH
```

Memory hook: **show, create, inspect.**

## Recipe 4: persistent system disk

```sh
target/release/synos-vm disk provision ./state/system.raw \
  --kernel build/bios/kernel.bin \
  --size 64M --boot-args "console=serial0"

target/release/synos-vm --system-disk ./state/system.raw --firmware bios --interactive
```

Memory hook: **provision first, boot from disk second.**

## Recipe 5: attach a data disk

```sh
target/release/synos-vm \
  --kernel build/bios/kernel.bin \
  --disk ./state/data.raw \
  --disk-size 64M \
  --disk-format raw \
  --disk-controller virtio-blk \
  --create-if-missing \
  --firmware bios --interactive
```

Memory hook: **path, size, format, controller, create.**

## Recipe 6: inspect safely

```sh
target/release/synos-vm disk list --system-disk ./state/system.raw
target/release/synos-vm disk inspect ./state/system.raw
target/release/synos-vm disk validate ./state/system.raw
```

Use `--read-only` for a shared base. Use copy-on-write for temporary mutation.

## Recipe 7: run deterministic evidence

```sh
./scripts/test-all.sh
```

Memory hook: **fast, bounded, recorded.**

## Recipe 8: run full validation

```sh
SYNOS_FULL_VALIDATION=1 ./scripts/full-validation.sh
```

Memory hook: **QEMU, cluster, hardware, fuzz, coverage, mutation, soak, release.**

## Recipe 9: inspect a cluster

```text
SHOW CLUSTER
SHOW CLUSTER/MEMBERS
SHOW CLUSTER/HEALTH
SHOW CLUSTER/RESOURCES
```

If a node is unsafe:

```text
DRAIN NODE node-7
FENCE NODE node-7 /CONFIRM
RECOVER NODE node-7 /CONFIRM
REJOIN NODE node-7 /CONFIRM
```

Memory hook: **inspect, drain, fence, reconcile, recover.**

## Recipe 10: build a safe service boundary

```text
request -> capability check -> descriptor bounds -> bounded work -> status
```

Never start with a raw pointer or an unbounded queue. Start with the request type, the capability, the limits, and the failure status.

## Recipe 11: add a feature

1. Define the state machine.
2. Define the owner and capability.
3. Define fixed limits.
4. Define restart and corruption behavior.
5. Add direct and boundary tests.
6. Add inventory IDs.
7. Add documentation and evidence.

## Recipe 12: debug a failure

```text
read result.json
read stderr.log
read stdout.log / serial log
check revision
replay seed or command
inspect state
```

Do not begin by rerunning a flaky command. First find the recorded evidence.

