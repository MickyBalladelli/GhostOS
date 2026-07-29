# SynOS Rust targets

`x86_64-unknown-synos.json` and `aarch64-unknown-synos.json` describe Ring 3
SynOS programs. They are position-independent, statically linked images with
abort-on-panic semantics.

The native platform contract is `synos_runtime::sys::synos`. A custom Rust
standard-library build uses its call gate for files, threads, waits, clocks,
memory mappings, and capability-mapped IPC. The process loader supplies the
gate and the initial capabilities; there is no ambient POSIX syscall table.

Until these target names ship with Rust, build `core`, `alloc`, and the SynOS
`std` port from `rust-src` with the corresponding JSON specification. Keep the
PAL revision pinned to the Rust channel in `rust-toolchain.toml`, because
`std::sys` is compiler-internal.
