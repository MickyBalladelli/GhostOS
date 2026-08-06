# Native Rust runtime

`synos-runtime` is the capability-backed PAL contract used by the SynOS Rust
`std` port. It has no ambient POSIX namespace and no host filesystem fallback.

The PAL provides:

- SynFS files, directories, metadata, links, deletion, and bounded path
  buffers;
- clock, arguments, environment pages, threads, thread exit, wait words,
  mutex parking, condition-variable parking, and TLS get/set;
- pipes, terminal input/output, random bytes, process spawn/exec/wait/cancel,
  process status, and memory protection;
- capability description for rights and object-kind discovery.

All calls use the fixed `synos-runtime` request/response ABI and capability
handles. Variable data crosses through capability-mapped `SharedBuffer` values.
The kernel validates buffer ownership, direction, bounds, and capability
rights before dispatch.

The runtime panic model is abort-only. SynOS does not promise host signal
semantics or stack unwinding. `PanicModel::Abort` is exported by
`sys::synos`; compiler and application profiles must use `panic=abort`.
Backtraces are an optional frame-provider interface over measured instruction
and stack pointers. Dynamic loading is `StaticOnly`; native images are fully
resolved before launch.

The fixed SynFS roots used by native compiler processes are:

```text
/system/toolchains/stage-2
/system/registries
/system/sources
/system/builds
/system/tmp
```

These roots replace host `HOME`, temporary-directory, registry, and linker
discovery assumptions inside SynOS. Host bootstrap tools may still select a
host Cargo executable; the in-guest PAL resolves compiler tools and files only
through capabilities.
