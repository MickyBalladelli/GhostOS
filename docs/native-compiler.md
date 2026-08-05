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

`synos-rustd` also has a signed `ToolchainManifest`. It requires Cargo, rustc,
and the linker, optionally requires build-script and proc-macro runners, then
executes the ordered plan through a process-runtime hook. Every tool receives
the bounded build request, and failed tools are fenced before the job fails.
The native loader and SynOS `std` PAL still need to provide that hook for true
in-guest execution.

Signed compiler output can now be launched through `synos-app`. The supervisor
validates the package instantiation receipt and entry point, authorizes the
manifest's requested capabilities against policy, and passes only the approved
capabilities plus the package payload identity to the process runtime. The
native executable-image loader still must map that payload before the process
can run on a booted SynOS instance.

The self-host path is represented by `SelfHostSession`. It submits and validates
the runtime build first, refuses unlocked or networked requests, then submits
the compiler build only after the runtime artifact is complete. Both results
must carry non-zero package and payload identities for the session to finish.

Reproducibility is checked with:

```sh
cargo run -p cargo-synos -- synos reproduce --target x86_64 --release
```

The command creates one fresh source workspace, builds it twice from clean
target state with `--locked --offline`, disables incremental state, remaps
workspace paths, and compares the final `.rlib`, `.rmeta`, `.a`, `.o`, and
`.so` artifacts by content ID. For `.rlib` files, it hashes code archive
members while ignoring Cargo metadata whose generated declaration order is
not stable.

The boot contract is now wired, but the actual native executable still needs
the SynOS `std` port, process loader, and service image described in
`TODO-compiler.md`.

For the complete host-side step, use `cargo-synos package` with
`--manifest-path`, `--locked`, `--offline`, `--key`, and `--output`.
