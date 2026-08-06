# SynOS Rust targets

`x86_64-unknown-synos.json` and `aarch64-unknown-synos.json` describe Ring 3
SynOS programs. They are position-independent, statically linked images with
abort-on-panic semantics.

The native platform contract is `synos_runtime::sys::synos`. A custom Rust
standard-library build uses its call gate for files, threads, waits, clocks,
memory mappings, random data, terminal I/O, process control, TLS, and
capability-mapped IPC. The process loader supplies the call gate and the
initial capabilities; there is no ambient POSIX syscall table. The PAL uses
abort-only panic semantics and static linking.

Until these target names ship with Rust, build `core`, `alloc`, and the SynOS
`std` port from `rust-src` with the corresponding JSON specification. Keep the
PAL revision pinned to the Rust channel in `rust-toolchain.toml`, because
`std::sys` is compiler-internal.

The `cargo-synos` extension automates the custom-target and `build-std` flags:

```sh
cargo install --path tools/cargo-synos
cargo synos build --target x86_64 --package my-service
```
