# Native Compiler

`cargo-synos compile-all` is the native-target build gate for the TODO
roadmap. It builds every production Ring 0 and Ring 3 library crate for a
SynOS target with `core`, `alloc`, position-independent code, and the bundled
Rust linker.

```sh
cargo run -p cargo-synos -- synos compile-all --target x86_64 --release
cargo run -p cargo-synos -- synos compile-all --target aarch64 --release
```

Excluded packages are host-side tools or test machines:

- `cargo-synos`
- `synos-compiler`
- `synos-test-support`
- `synos-uefi`
- `synos-vm`

The compiler driver runs on the build host. The output is native SynOS code.
Self-hosting Rust compilation inside a running SynOS instance needs a SynOS
`std` port, process loader, dynamic library support, compiler package, and a
filesystem-backed compiler service. Those are separate runtime work, not a
property that can be claimed from a host Cargo command.
