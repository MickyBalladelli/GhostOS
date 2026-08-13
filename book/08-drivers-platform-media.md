# 8. Drivers, Platform I/O, Power, and Media

The kernel does not absorb every device driver. Hardware access is split between capability-controlled platform I/O and Ring 3 driver services.

## Asynchronous platform I/O

`synos-platform-io` provides fixed-capacity request and completion queues. The lifecycle is:

```text
submit -> dispatch -> complete
   |                    |
 cancel                poll
```

Requests are generation-checked. Buffers are capability-mapped. A driver cannot write a caller’s memory merely because a numeric address looks plausible.

The same descriptor rules cover storage and multi-plane audio/video operations such as present, capture, encode, and decode.

## Legacy PC drivers

`synos-legacy-pc-drivers` provides heap-free Ring 3 building blocks for:

- conventional PCI enumeration;
- AHCI and NVMe discovery and command preparation;
- Intel E1000-family Ethernet;
- Realtek RTL8169-family Ethernet;
- DMA queues and descriptor rings.

Port and MMIO access remain controlled by the platform service.

## Power and ACPI

`synos-power` validates the ACPI RSDP, RSDT/XSDT, FADT, and DSDT checksums without allocation. It reads reset and fixed-event registers, extracts `_S5` shutdown values, and applies thermal thresholds with hysteresis.

Power operations include:

- ACPI S3 suspend and firmware-assisted resume when `_S3_` is advertised;
- `SHUTDOWN`;
- `REBOOT`;
- physical power-button handling;
- emergency thermal shutdown;
- legacy reset fallback.

The safe order is: quiesce work, flush persistent state, notify services, then invoke platform power control.

## Storage device lifecycle

NVMe namespaces and CXL pools expose online, draining, and removed states. A draining NVMe namespace refuses new SynFS allocations and waits for existing users. A CXL pool refuses new memory leases while draining. Final detach requires controller or decoder quiescence.

## Media and compute devices

The compute and media path uses shared multi-plane buffers. A video or audio operation declares each plane, offset, length, format, and direction. The service validates alignment and bounds before touching the data.

This avoids a common trap: treating a frame as one flat blob when the hardware expects separate planes.

## Easy example: safe device removal

```text
ONLINE -> DRAINING -> QUIESCE -> RELEASE LEASES -> REMOVED
```

If a lease remains, removal stops. If a request is in flight, the service completes, cancels, or reports failure before the device leaves the graph.
