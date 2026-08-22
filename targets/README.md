# GhostOS Rust targets

`x86_64-unknown-ghostos.json` and `aarch64-unknown-ghostos.json` describe Ring 3
GhostOS programs. They are position-independent, statically linked images with
abort-on-panic semantics.

The native platform contract is `ghostos_runtime::sys::ghostos`. A custom Rust
standard-library build uses its call gate for files, threads, waits, clocks,
memory mappings, random data, terminal I/O, process control, TLS, and
capability-mapped IPC. The process loader supplies the call gate and the
initial capabilities; there is no ambient POSIX syscall table. The PAL uses
abort-only panic semantics and static linking.

Until these target names ship with Rust, build `core`, `alloc`, and the GhostOS
`std` port from `rust-src` with the corresponding JSON specification. Keep the
PAL revision pinned to the Rust channel in `rust-toolchain.toml`, because
`std::sys` is compiler-internal.

The `cargo-ghostos` extension automates the custom-target and `build-std` flags:

```sh
cargo install --path tools/cargo-ghostos
cargo ghostos build --target x86_64 --package my-service
```

The acceptance gate admits x86_64 first. Aarch64 stays a cross-build target
until its GhostOS runtime, linker, process loader, and boot acceptance evidence
all exist; a successful Aarch64 compile alone is not compatibility proof.
