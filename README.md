# GhostOS

GhostOS is a capability-based, `no_std` operating-system project. A small
kernel owns the hard boundaries. User-space services own drivers, storage,
networking, policy, applications, and AI. The virtual machine makes the whole
system testable.

The [system book](book/README.md) is the long-form map of the repository.
This README is the short path to a running image.

## Quickstart

Install the Rust targets and LLVM tools listed in `rust-toolchain.toml`, plus
`clang`.

Build a BIOS disk image:

```sh
./scripts/build-bios-image.sh
```

The image is written to `build/bios/ghostos-bios.img`.

Run the interactive BIOS VM (builds the release `ghostos-vm` first if needed):

```sh
./start-ghostos.sh
```

Named VMs keep separate persistent disks:

```sh
./start-ghostos.sh vm1
./start-ghostos.sh --new
./start-ghostos.sh --no-passkey-web
```

`start-ghostos.sh` creates a bootable `system.raw` (or `<name>-system.raw`)
and keeps `data.raw` as a data disk. `--new` discards disk changes when the
VM exits. `--no-passkey-web` is serial-only login; see
[first boot](docs/first-boot.md).

Writable disks create a `<image>.ghostos.lock` ownership marker. If start
fails with `disk is already locked` after a crashed VM:

```sh
./target/release/ghostos-vm disk lock ./virtual_machine/state/system.raw
./target/release/ghostos-vm disk recover-lock ./virtual_machine/state/system.raw
```

Build the BIOS image, the VM, and host tests in one step:

```sh
./scripts/build-and-test.sh
```

Build the UEFI application with `./scripts/build-uefi-loader.sh`. USB, dual
boot, and Docker workflows are in
[operations and lifecycle](book/15-operations-and-lifecycle.md).

## Test

```sh
cargo test
```

The test contract, tiers, evidence format, and feature inventory are in
[`docs/testing.md`](docs/testing.md).

## Documentation

- [System book](book/README.md)
- [First boot](docs/first-boot.md)
- [Login methods](docs/login.md)
- [Central identity and VM fleet](docs/central-identity.md)
- [Account management](docs/account-management.md)
- [Compatibility matrix](docs/compatibility-matrix.md)
- [Hardware support](docs/hardware-support.md)
- [Test contract](docs/testing.md)

Check docs against source metadata:

```sh
python3 scripts/validate-documentation.py
```
