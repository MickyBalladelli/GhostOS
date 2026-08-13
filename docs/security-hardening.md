# Security hardening

Native position-independent images receive entropy-seeded address hints from the kernel. The
current statically linked bootstrap targets cannot relocate the kernel, so KASLR is enabled only
when a future boot target supplies relocation support; the kernel does not claim a fake slide.

Every user mapping rejects simultaneous write and execute permissions. Context installation also
requires an executable entry point and a writable, non-executable stack. x86_64 boot services are
not started on hardware without NX support; the kernel falls back to its Ring 0 shell instead.
Their code is read-only, while state, stack, and MMIO mappings are non-executable.

On x86_64, the kernel enables SMEP and SMAP when CPUID reports them. All kernel access to Ring 3
request, response, filesystem, terminal, and random buffers passes through bounded SMAP access
windows. Built-in Ring 3 C services use stack canaries seeded by the kernel and compiler
control-flow landing pads.

Private key bytes remain behind the `synos-shield` `KeyProvider` boundary. The kernel-facing
authority accepts only hardware-backed or isolated-service providers, exposes opaque handles, and
has no private-key import, export, logging, snapshot, or crash-capsule path.

`fuzz/fuzz_targets/syscall_boundary.rs` feeds arbitrary syscall headers and cryptographic
capabilities into the production validators. Privilege-escalation tests cover token mutation and
rights amplification. Confused-deputy tests cover namespace, resource, federation, and operation
mix-ups with otherwise valid capabilities.
