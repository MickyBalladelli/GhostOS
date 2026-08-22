# GhostOS platform qualification

These profiles separate repeatable platform validation from development claims.
A platform passes only when `scripts/qualify-platform.sh` accepts evidence
captured from that exact machine or emulated topology.

## One physical x86_64 boot

Capture the COM1 output from one supported x86_64 machine. The capture must
reach the Ring 3 shell; QEMU, KVM, and HVF output is not hardware evidence.
Write `inventory.txt` in `key=value` form using the fields below:

```text
environment=bare-metal
hypervisor=none
architecture=x86_64
boot_mode=bios
console=com1
machine_id=<local non-secret machine label>
firmware_vendor=<firmware vendor>
firmware_version=<firmware version>
motherboard=<board model>
cpu=<CPU model>
memory_mib=<RAM in MiB>
storage=<boot disk model>
nic=<NIC model and PCI ID>
```

After shutting the machine down, package the serial log, inventory, and exact
boot artifact used for that boot:

```sh
python3 scripts/record-hardware-boot-evidence.py \
  --evidence-dir build/test-evidence/<run-id>/hardware-boot \
  --serial-log /path/to/node.serial.log \
  --inventory /path/to/inventory.txt \
  --artifact build/bios/ghostos-bios.img \
  --boot-mode bios \
  --operator <operator-id> \
  --revision "$(git rev-parse HEAD)" \
  --started-at 2026-08-13T09:00:00Z \
  --ended-at 2026-08-13T09:05:00Z
```

Validate the retained bundle and its hashes with:

```sh
./scripts/qualify-platform.sh hardware-boot build/test-evidence/<run-id>/hardware-boot
```

The validator requires the kernel bootstrap, x86_64 hardware discovery,
`ghostos-init` Ring 3 handoff, and `ghostos-shell` readiness markers. It also
rejects panic output, a hypervisor declaration, missing artifact hashes, and
an inventory that does not identify the target machine.

## Two consumer PCs

Use two x86_64 PCs with supported Intel E1000-family or Realtek RTL8169-family
Ethernet adapters. Connect both machines to one isolated Ethernet switch.
Disable Wi-Fi and other cluster links so page traffic cannot escape the test
network.

1. Build `build/bios/ghostos-bios.img` and write it to two spare USB drives.
2. Boot both PCs and capture COM1 output as `node-1.serial.log` and
   `node-2.serial.log`.
3. Record NIC model, PCI IDs, CPU, RAM, boot mode, and the words `bare-metal`
   in `inventory.txt`.
4. Run a remote-page read, a write/invalidation cycle, and sustained
   heartbeats. Save service output as `fabric.log`.
5. Power off the page owner while traffic is active. Save recovery output as
   `failover.log`.
6. Run:

   ```sh
   ./scripts/qualify-platform.sh legacy-two-node evidence/legacy-two-node
   ```

The gate requires both boots, NIC recognition, heartbeat traffic, a remote
page operation, and completed failover.

## Local QEMU cluster

Build the BIOS image, then launch the sandbox:

```sh
./scripts/build-bios-image.sh
./scripts/qemu-cluster.sh
```

The launcher creates two guests by default. Each guest receives:

- a unique E1000 NIC on one QEMU multicast Ethernet bus;
- a private CXL Type-3 volatile-memory endpoint and CXL fixed memory window;
- one `ivshmem-plain` mapping backed by the same host file in every guest;
- separate serial, PID, and command evidence files under
  `build/qemu-cluster`.

Set `GHOSTOS_CLUSTER_NODES`, `GHOSTOS_GUEST_MEMORY`, `GHOSTOS_CXL_MEMORY`,
`GHOSTOS_SHARED_MEMORY`, or `GHOSTOS_QEMU_ACCEL` to change the topology.

Inject one node loss from another terminal:

```sh
./scripts/qemu-cluster-fail-node.sh 2
```

Copy fabric and failover service logs into the run directory, then run:

```sh
./scripts/qualify-platform.sh qemu-cluster build/qemu-cluster
```

## Enterprise CXL switched target

The minimum target has two hosts, one CXL switch, two Type-3 endpoints, and
mirrored memory reachable after either host is removed. Capture:

- PCIe/CXL topology and firmware revisions in `inventory.txt`;
- every discovered endpoint and committed HDM decoder in `fabric.log`;
- a remote page read/write and migration in `fabric.log`;
- physical endpoint or host removal with mirror redirection in `failover.log`;
- serial output from two GhostOS hosts.

Qualify it with:

```sh
./scripts/qualify-platform.sh enterprise-cxl evidence/enterprise-cxl
```

## Enterprise PCIe or NVLink GPU target

The minimum target has two peer-accessible accelerators and VRAM registered in
the global fabric map. Record PCIe or NVLink topology in `inventory.txt`, VRAM
pool and peer-memory discovery in `fabric.log`, then perform the same two-host
heartbeat, migration, and failover capture.

Qualify it with:

```sh
./scripts/qualify-platform.sh enterprise-gpu evidence/enterprise-gpu
```

Passing one profile does not imply another profile passed. Keep evidence with
the hardware serial numbers and firmware versions used for that run.
