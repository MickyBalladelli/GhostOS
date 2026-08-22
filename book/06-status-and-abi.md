# 6. The Status and ABI Contracts

Many operating systems fail at the edges between subsystems. GhostOS gives those edges names.

## `ghostos-status`

The status crate defines a common 32-bit condition layout. It preserves the OpenVMS convention that odd values indicate success while exposing structured fields for:

- facility;
- numeric code;
- severity;
- flags;
- message or condition identity.

The same status can cross kernel, daemon, shell, SDK, and client boundaries without being flattened into a host error string.

## Why odd-value success matters

OpenVMS-style conditions let a caller distinguish success with information from failure:

```text
SUCCESS        -> operation worked
INFO            -> worked, but there is useful detail
WARNING         -> worked with caution
ERROR           -> operation failed
```

The exact encoded value is less important than preserving the category through every layer.

## Runtime ABI

`ghostos-runtime` is the user-facing system-call contract. It validates operation numbers, arguments, buffer directions, shared-region bounds, and descriptor lifecycles. Unknown operations return a stable status rather than falling into an unchecked default.

The complete native contract is in [`docs/abi.md`](../docs/abi.md). The current
syscall schema is version 1, with a fixed 64-byte request and 40-byte response.
The request carries the schema version, and the kernel rejects a mismatched
version before dispatch.

The ABI should be treated like a wire protocol:

1. identify the operation;
2. validate fixed fields;
3. validate referenced memory;
4. check the caller’s capability;
5. perform bounded work;
6. write the result only into an authorized output region;
7. return a status and completion metadata.

## Boot protocol

`ghostos-boot-protocol` is the cross-architecture handoff crate. Keep it boring. It should contain representations that both firmware and kernel can agree on, alignment and version constants, memory kinds, and framebuffer formats.

## Versioning rule

Unknown fields may be ignored only when the record length and version rules say that is safe. A future field must not shift the location of old fields. A protocol change needs:

- a version decision;
- compatibility tests;
- malformed input tests;
- a migration or refusal path;
- evidence in the inventory.

## Easy example: status travels upward

```text
NVMe controller: queue full
        -> storage service: BUSY, retry_after = 500 us
        -> shell: device busy; retrying
        -> client SDK: typed Busy result
```

The user sees a helpful message. The machine keeps the precise condition.

## User-space package ABI

Signed Ring 3 applications use the versioned `SYNBNDL1` inner package and
`SYNAPP01` application envelope. Both are version 1 today, and application
metadata uses schema 1. The package daemon verifies content IDs, dependencies,
trusted-key signatures, and reserved bytes before installation. The supervisor
then matches signed target and resource metadata before launch. The complete
record layout and compatibility rules are in [`docs/package-abi.md`](../docs/package-abi.md).
