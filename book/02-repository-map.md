# 2. How the Repository Is Organized

The repository is a Cargo workspace. The root `Cargo.toml` lists the kernel, boot loader, support crates, and `virtual_machine` as workspace members. The `fuzz` directory is a separate Cargo-fuzz workspace because fuzz binaries have special build behavior.

## Directory map

```text
GhostOS/
├── boot/
│   ├── bios/                  BIOS assembly and linker inputs
│   ├── grub/                  GRUB-related boot assets
│   └── uefi/                  UEFI Rust loader
├── clients/apple/             Swift client and control UI
├── crates/                    Ring 3 services and reusable libraries
├── docs/                      Test and configuration contracts
├── fuzz/                      LibFuzzer targets and corpora
├── kernel/                    no_std microkernel and architecture code
├── platforms/                 qualification profiles and evidence rules
├── scripts/                   build, test, QEMU, coverage, and release tools
├── targets/                   custom target specifications
├── tools/cargo-ghostos/         Cargo subcommand support
├── virtual_machine/           Rust VM, devices, firmware, and tests
└── book/                      this book
```

## Workspace layers

The names tell a useful story:

| Layer | Main packages | Job |
| --- | --- | --- |
| Boot | `ghostos-boot-protocol`, `ghostos-uefi` | Define and deliver the machine handoff |
| Kernel | `ghostos-kernel` | Tasks, memory, IPC, capabilities, interrupts, power |
| Core services | `ghostos-ghostfs`, `ghostos-fsd`, `ghostos-runtime`, `ghostos-ipc` | Files, namespaces, ABI, and service transport |
| Hardware | `ghostos-platform-io`, `ghostos-legacy-pc-drivers`, `ghostos-power` | Device access and lifecycle |
| Network | `ghostos-netd`, `ghostos-http`, `ghostos-client-sdk` | Packets, sockets, web, and clients |
| Operations | `ghostos-shell`, `ghostos-script`, `ghostos-status`, `ghostos-observability` | Human control and common records |
| Security | `ghostos-auth`, `ghostos-shield`, `ghostos-confidential`, `ghostos-auditd` | Identity, runtime protection, attestation, audit |
| Data | `ghostos-rms`, `ghostos-pkg`, `ghostos-backup`, `ghostos-storaged`, `ghostos-kvd` | Records, packages, backup, remote storage, cache |
| Cluster | `ghostos-fabric`, `ghostos-actors`, `ghostos-balancerd`, `ghostos-mesh`, `ghostos-time-sync` | Memory, actors, placement, federation, clocks |
| AI | `ghostos-compute`, `ghostos-llm`, `ghostos-inference`, `ghostos-agentd`, `ghostos-agent-bridge` | Tensors, models, inference, memory, tools |
| Tooling | `ghostos-debug`, `ghostos-inspect`, `ghostos-top`, `ghostos-replay`, `ghostos-heal` | Diagnose, observe, replay, recover |
| Test machine | `ghostos-vm`, `ghostos-test-support` | Emulate the machine and make evidence repeatable |

## Read code by contract

When a feature crosses layers, follow the type that carries its boundary:

- Boot: `BootInfo`.
- Memory: `MemoryRegion`, frame and page-table types.
- Authority: `CapabilityHandle`, `Rights`, capability objects.
- Messages: IPC request/completion records and shared-region descriptors.
- Storage: GhostFS generations, manifests, and block descriptors.
- Networking: socket capabilities, packet buffers, and protocol frames.
- Distributed state: node IDs, epochs, leases, and fencing tokens.
- Errors: `ghostos-status` condition values.

This is faster than reading every module in directory order. Find the contract, then find the producer, consumer, validator, and test.

## Naming pattern

Most package names start with `ghostos-`. The crate name and the feature name may differ:

- directory `crates/llm-runtime` contains package `ghostos-llm`;
- directory `crates/logd` contains package `ghostos-logd`;
- directory `crates/ghostfs` contains package `ghostos-ghostfs`;
- `ghostos-script` and `ghostos-shell` keep their shorter historical names.

Use `cargo metadata` when unsure. Do not infer a package name from a directory.

## A first code-reading route

```text
README.md
  -> Cargo.toml
  -> crates/boot-protocol/src/lib.rs
  -> kernel/src/lib.rs
  -> kernel/src/task.rs
  -> kernel/src/ipc.rs
  -> kernel/src/capability.rs
  -> crates/ghostfs/src/lib.rs
  -> crates/ghostos-shell/src/lib.rs
  -> virtual_machine/src/lib.rs
```

That route shows the architecture without drowning in feature modules.

