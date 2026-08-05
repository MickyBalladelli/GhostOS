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

The current service boundary is represented by `synos-rustd`. Its requests are
fixed-size and include the source root, manifest, target, profile, lock policy,
network policy, and resource limits. The default policy denies network access.
During user-space boot, `CompilerServiceBoot` verifies the package gate,
registers the service with init, and starts it under the compiler capability
profile. `CompilerServiceHealthCheck` makes a system update fail and roll back
when the service is not running or the active root is not bound to that
package. The host driver accepts matching `--locked` and `--offline` flags and
can write a signed package atomically after a successful build. Its
`compile_synfs` path validates the source root and manifest through SynFS,
copies the bounded project tree into an isolated Cargo workspace, and compiles
that snapshot with the same target and lock policy.

The boot contract is now wired, but the actual native executable still needs
the SynOS `std` port, process loader, and service image described in
`TODO-compiler.md`.

For the complete host-side step, use `cargo-synos package` with
`--manifest-path`, `--locked`, `--offline`, `--key`, and `--output`.
