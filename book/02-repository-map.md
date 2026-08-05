# 2. How the Repository Is Organized

The repository is a Cargo workspace. The root `Cargo.toml` lists the kernel, boot loader, support crates, and `virtual_machine` as workspace members. The `fuzz` directory is a separate Cargo-fuzz workspace because fuzz binaries have special build behavior.

## Directory map

```text
SynOS/
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
├── tools/cargo-synos/         Cargo subcommand support
├── virtual_machine/           Rust VM, devices, firmware, and tests
└── book/                      this book
```

## Workspace layers

The names tell a useful story:

| Layer | Main packages | Job |
| --- | --- | --- |
| Boot | `synos-boot-protocol`, `synos-uefi` | Define and deliver the machine handoff |
| Kernel | `synos-kernel` | Tasks, memory, IPC, capabilities, interrupts, power |
| Core services | `synos-synfs`, `synos-fsd`, `synos-runtime`, `synos-ipc` | Files, namespaces, ABI, and service transport |
| Hardware | `synos-platform-io`, `synos-legacy-pc-drivers`, `synos-power` | Device access and lifecycle |
| Network | `synos-netd`, `synos-http`, `synos-client-sdk` | Packets, sockets, web, and clients |
| Operations | `syn-shell`, `syn-script`, `synos-status`, `synos-observability` | Human control and common records |
| Security | `synos-auth`, `synos-shield`, `synos-confidential`, `synos-auditd` | Identity, runtime protection, attestation, audit |
| Data | `synos-rms`, `synos-pkg`, `synos-backup`, `synos-storaged`, `synos-kvd` | Records, packages, backup, remote storage, cache |
| Cluster | `synos-fabric`, `synos-actors`, `synos-balancerd`, `synos-mesh`, `synos-time-sync` | Memory, actors, placement, federation, clocks |
| AI | `synos-compute`, `synos-llm`, `synos-inference`, `synos-agentd`, `synos-agent-bridge` | Tensors, models, inference, memory, tools |
| Tooling | `synos-debug`, `synos-inspect`, `synos-top`, `synos-replay`, `synos-heal` | Diagnose, observe, replay, recover |
| Test machine | `synos-vm`, `synos-test-support` | Emulate the machine and make evidence repeatable |

## Read code by contract

When a feature crosses layers, follow the type that carries its boundary:

- Boot: `BootInfo`.
- Memory: `MemoryRegion`, frame and page-table types.
- Authority: `CapabilityHandle`, `Rights`, capability objects.
- Messages: IPC request/completion records and shared-region descriptors.
- Storage: SynFS generations, manifests, and block descriptors.
- Networking: socket capabilities, packet buffers, and protocol frames.
- Distributed state: node IDs, epochs, leases, and fencing tokens.
- Errors: `synos-status` condition values.

This is faster than reading every module in directory order. Find the contract, then find the producer, consumer, validator, and test.

## Naming pattern

Most package names start with `synos-`. The crate name and the feature name may differ:

- directory `crates/llm-runtime` contains package `synos-llm`;
- directory `crates/logd` contains package `synos-logd`;
- directory `crates/synfs` contains package `synos-synfs`;
- `syn-script` and `syn-shell` keep their shorter historical names.

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
  -> crates/synfs/src/lib.rs
  -> crates/syn-shell/src/lib.rs
  -> virtual_machine/src/lib.rs
```

That route shows the architecture without drowning in feature modules.

