# GhostOS syscall ABI

This is the native Ring 3 system-call contract. The machine-readable source is
[`abi/ghostos-abi.toml`](../abi/ghostos-abi.toml). It also owns the shared RPC
registry and status values. Rust and Swift bindings are generated from it.

Current contract:

| Field | Value | Meaning |
| --- | ---: | --- |
| `ABI_SCHEMA_VERSION` | `1` | Wire version carried in every request. The kernel accepts this value exactly. |
| `ABI_REVISION` | `14` | Monotonic generated-registry revision. It identifies the binding set; it is not a negotiation value. |
| Request size | `64` bytes | Fixed `#[repr(C)]` request record. |
| Response size | `40` bytes | Fixed `#[repr(C)]` response record. |

`ABI_SCHEMA_VERSION` changes when an existing encoding, operation number,
argument meaning, validation rule, status meaning, or return value changes.
Such a change needs a new ABI version and a new binding set. `ABI_REVISION`
records source evolution and must not be used to claim compatibility.

## Entry points

The caller passes pointers to one request and one response. Both records must
be naturally aligned and must live in the caller's readable/writable user
memory. The kernel validates the ranges before reading or writing them.

| Architecture | User entry | Request pointer | Response pointer |
| --- | --- | --- | --- |
| x86_64 | DPL 3 interrupt gate, vector `0x80` | `RDI` | `RSI` |
| AArch64 | `svc` exception | `x0` | `x1` |
| RISC-V 64 | `ecall` from user mode | `a0` | `a1` |

The common entry is `ghostos_call_gate_dispatch`. A successful `SleepUntil`
call may cause the scheduler to block the current thread before returning.

## Fixed records

All integer fields are little-endian, and all unused fields must be zero.

### Request

| Offset | Size | Field |
| ---: | ---: | --- |
| `0` | 2 | `operation: u16` |
| `2` | 2 | `abi_version: u16`, must be `1` |
| `4` | 2 | `flags: u16`, operation-specific |
| `6` | 2 | `reserved: u16`, must be zero |
| `8` | 8 | `capability: u64`, zero or a generation-checked capability token |
| `16` | 48 | `arguments: u64[6]`, operation-specific |

### Response

| Offset | Size | Field |
| ---: | ---: | --- |
| `0` | 4 | `status: u32`, a raw `ghostos-status` value |
| `4` | 4 | `flags: u32`, currently zero |
| `8` | 32 | `values: u64[4]`, operation-specific return values |

Status success is tested with `Status::is_success()`; callers must not treat
zero as success. A rejected ABI version returns `PROTOCOL_MISMATCH` before the
operation is dispatched. A malformed record returns `INVALID_ARGUMENT`.

## Dispatch rules

The entry path checks, in order:

1. request and response pointers, alignment, and user ranges;
2. ABI version, operation number, and reserved fields;
3. operation flags and argument shape;
4. shared-buffer bounds and read/write direction;
5. capability validity and required rights;
6. the bounded operation itself.

Failure writes a response and performs no operation-specific mutation. Unknown
operation numbers are invalid. Capability `0` means no capability; a non-zero
capability must contain a non-zero generation in its upper 32 bits.

For buffer operations, `arguments[0..4]` is a shared-buffer descriptor:

| Argument | Meaning |
| ---: | --- |
| `0` | shared region ID (`u32`) |
| `1` | byte offset (`u32`) |
| `2` | byte length (`u32`) |
| `3` | writable bit (`0` or `1`) |

`arguments[4]` and `arguments[5]` then carry an operation's offset, length,
or continuation value. The typed `ghostos-runtime` API is the safe way to build
these descriptors.

## Operation registry

Numbers never change meaning. New operations use a new number and must not
reuse a removed number.

| Number | Operation | Number | Operation | Number | Operation |
| ---: | --- | ---: | --- | ---: | --- |
| 1 | `Yield` | 18 | `SynFsRmdir` | 35 | `PipeRead` |
| 2 | `ClockNow` | 19 | `SynFsLink` | 36 | `PipeWrite` |
| 3 | `Wait` | 20 | `SynFsList` | 37 | `PipeClose` |
| 4 | `Wake` | 21 | `SynFsLinks` | 38 | `ThreadTlsGet` |
| 5 | `ThreadSpawn` | 22 | `SynFsDelete` | 39 | `ThreadTlsSet` |
| 6 | `ThreadJoin` | 23 | `RandomGet` | 40 | `ArgumentsRead` |
| 7 | `ThreadExit` | 24 | `TerminalRead` | 41 | `EnvironmentRead` |
| 8 | `MemoryMap` | 25 | `TerminalWrite` | 42 | `ServiceReady` |
| 9 | `MemoryUnmap` | 26 | `ProcessSpawn` | 43 | `ServiceHeartbeat` |
| 10 | `IpcMap` | 27 | `ProcessExec` | 44 | `SystemInfo` |
| 11 | `IpcNotify` | 28 | `ProcessWait` | 45 | `ShellPoll` |
| 12 | `SynFsOpen` | 29 | `ProcessCancel` | 46 | `RealtimeNow` |
| 13 | `SynFsClose` | 30 | `ProcessExit` | 47 | `SleepUntil` |
| 14 | `SynFsRead` | 31 | `ProcessStatus` | 48 | `SynFsMap` |
| 15 | `SynFsWrite` | 32 | `MemoryProtect` | 49 | `SynFsUnmap` |
| 16 | `SynFsMetadata` | 33 | `CapabilityQuery` | | |
| 17 | `SynFsMkdir` | 34 | `PipeCreate` | | |
| 64 | `WatchdogDiagnostics` | | | | |

`WatchdogDiagnostics` is shell-only. `arguments[0]` is `0` for status, `1` to
enable diagnostic output, or `2` to disable it; all other arguments must be
zero. The response returns enabled state, service timeout in microseconds,
and service-role capacity in `values[0..3]`.

The operation enum and typed argument construction live in
[`crates/abi/src/generated.rs`](../crates/abi/src/generated.rs) and
[`crates/runtime`](../crates/runtime). An operation may be allocated in the
registry before every runtime backend supports it; an unsupported operation
returns `INVALID_ARGUMENT`.

## Updating the ABI

Edit the TOML source first, then regenerate the checked-in bindings:

```sh
python3 tools/generate_abi.py
```

Check for stale generated output with:

```sh
python3 tools/generate_abi.py --check
```

For every ABI change, update this page and the compatibility matrix. Preserve
old operation numbers and field offsets. If old and new records cannot be
handled by the same decoder, bump `schema_version`; do not silently translate
or reinterpret bytes. There is no in-place syscall ABI migration: ship a
matching kernel and user-space binding set.
