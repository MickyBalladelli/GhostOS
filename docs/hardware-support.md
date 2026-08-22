# Physical hardware support

This page describes the implemented legacy x86_64 hardware path. Anything not
listed as supported must be treated as unsupported.

## Storage

| Hardware | Status | Limits |
| --- | --- | --- |
| AHCI SATA disks | Boot and block I/O | The early root mount requires 512-byte logical sectors and a GhostOS system-disk v1 image. The first AHCI disk with a valid redundant manifest is used. ATAPI is not supported. |
| NVMe namespaces | Block I/O | Read, write, flush, discard, queue submission, and completion are implemented. Firmware or the driver service must configure the admin and I/O queues. Early root mounting from NVMe is not yet supported. |
| USB mass storage | Driver-class support | Bulk-Only Transport with SCSI READ(10), WRITE(10), SYNCHRONIZE CACHE, and UNMAP is supported over a configured USB bulk transport. UAS is not supported. |

GhostFS uses two complete generation banks. Data and the type map are written
before the inactive bank's superblock, then the device flush command is used as
the durability fence. Mount selects the newest checksum-valid committed bank,
so an interrupted write falls back to the previous generation.

The physical system-disk reader accepts `SYNOSDSK`/`SYNMANIF` version 1 and
mounts the aligned GhostFS system extent. Service images are read from
`/system/services`, checked against the `SYNSVC01` manifest, and copied into
their Ring 3 address spaces. Provision them with repeatable options:

```sh
ghostos-vm disk provision system.raw --kernel kernel.bin \
  --service 1=ghostos-init.bin \
  --service 2=ghostos-fsd.bin \
  --service 9=ghostos-shell.bin
```

Roles are address-space IDs `1` through `14`. Missing roles use the kernel's
embedded bootstrap image.

## USB and input

The xHCI path supports one directly attached HID boot keyboard. USB hubs,
xHCI controllers requiring scratchpad buffers, isochronous transfers, and
EHCI/OHCI/UHCI controllers are unsupported. The shared USB class layer decodes
HID keyboard and mouse reports and runs USB mass-storage BOT commands when a
host driver supplies configured bulk endpoints.

PS/2 keyboard and three-byte PS/2 mouse packets are supported. The mouse path
reports three buttons and relative X/Y movement. USB mouse report decoding is
implemented, but automatic xHCI mouse attachment is not.

## Platform limits

- Bare-metal device discovery is x86_64 PCI configuration mechanism 1.
- AHCI is the only automatic physical root-volume transport today.
- RAID firmware, SAS, IDE, eMMC, SD, Thunderbolt storage, multipath, hot-plug
  root disks, and encrypted boot volumes are unsupported.
- Hardware outside these limits may still work through a future Ring 3 driver;
  it is not part of the current compatibility promise.
