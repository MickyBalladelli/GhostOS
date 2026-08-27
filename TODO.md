# GhostOS improvement list

Open work from a full-tree review of the kernel, Ring 3 boot services, GhostFS/fsd, VM, workspace, and test machinery. Items are independent unless noted. Do not mark an item done without a regression test that would have failed before the change.

The SynOS rename is largely complete in crate and binary names. Remaining SynOS strings are on-disk or on-wire compatibility surfaces; do not blindly replace them.

---

## Bug fixes

- [x] Give `login.c` and `service.c` the same syscall `call()` contract as `shell.c`.
  `userspace/boot-services/login.c` and `service.c` still run `int $0x80` with no `+m` outputs for `request`/`response` and only clobber `rax`. Clang can keep a zeroed `response` in registers after the kernel writes the stack slot. `shell.c` already lists `+m` and the volatile registers. Copy that pattern so login/bridge reads and service probes cannot observe stale status.

- [x] Make `CREATE`, `TYPE`/`CAT`, `MKDIR`, `RMDIR`, and `DELETE` use absolute paths.
  `userspace/boot-services/shell.c` `execute_line` only calls `make_absolute_path` for `DIRECTORY`/`DIR`/`LS`. GhostFS stores names as `/packages`, `/data`, … so `mkdir foo` and `type note` miss the objects `dir` just listed. Apply the same absolute-path step (and a real error when the path is too long; `make_absolute_path` currently returns without adding `/` if `path_length >= 255`).

- [x] Split LIST path from LIST output instead of treating leftover records as `/`.
  `crates/fsd/src/daemon.rs` `Operation::List` now lists the root when the buffer prefix does not start with `/`. That hides the leftover-record footgun for the boot shell, but any other LIST caller that forgets to rewrite the path will silently list `/`. `Operation::SnapshotList` still parses the prefix strictly. Give LIST a dedicated path length or a path region that is not overwritten by the encoded page, and make SnapshotList match.

- [x] Stop falling through to a blank in-memory filesystem when AHCI mount fails.
  `kernel/src/boot_services.rs` logs `[fsprobe] boot: AHCI MOUNT FAILED` and then `unwrap_or_else(SynFs::new)`. Accounts, passkeys, and packages on the system disk disappear with no recovery prompt. Fail boot or enter an explicit recovery mode when a system volume was expected (`start-ghostos.sh` always provisions `system.raw`).

- [x] Resolve Ring 3 service images by the kernel build that produced them.
  `start-ghostos.sh` uses `find … -name ghostos-shell.bin -print -quit` under `target/x86_64-unknown-none/release/build`. The first match can be an old hash directory. Staleness is keyed only on `build/bios/kernel.bin` mtime. Pin images to `build/kernel-ring3` from the `ghostos-kernel` build script (a stable copy of that build's `OUT_DIR`), and refresh services whenever boot-service sources change.

- [x] Return `AuthError` for non-UTF-8 usernames instead of panicking.
  `crates/auth/src/identity.rs` uses `core::str::from_utf8(...).unwrap()` while checking reserved names. Invalid stored bytes should be `InvalidRecord`, not a `no_std` panic. `as_str()` still uses `expect` on the same invariant; keep one fallible conversion at the boundary.

- [x] Add a guest LIST/`dir` test on the release kernel, not only host fsd tests.
  Host tests in `crates/fsd/src/tests.rs` and `crates/ghostfs/tests/list_root.rs` pass in debug. The guest shell is compiled `-O2` and the kernel is `lto = true` plus `opt-level = "z"`. The empty-`dir` / second-`dir` NotFound failure only showed up in QEMU. Drive serial through login and `dir` twice against `build/bios/kernel.bin`.

- [x] Repair the VM public-API inventory so quality gates are truthful.
  `python3 scripts/generate-vm-inventory.py --check` reports missing named tests (passkey, DHCP, system-disk refresh, and others). `scripts/validate-vm-quality.py` also flags stale `src/net/dhcp.rs`. `scripts/test-all.sh` runs this tier; keep `virtual_machine/tests/inventory.toml` in sync or stop claiming the gate is green.


---

## Quality

- [x] Unify the three shells.
  `userspace/boot-services/shell.c` is the logged-in Ring 3 prompt (`ghostos-shell.bin`). `crates/ghostos-shell` is a separate Rust parser/editor with DIRECTORY/EDIT/network routes. `kernel/src/shell.rs` is a Ring 0 operator shell that uses the Rust crate. Command sets already drift (`dir` vs `DIRECTORY`, no `EDIT` in C). Pick one userspace shell and keep the kernel path as a debugger, or generate the C command table from the Rust crate.

- [x] Document or merge the two filesystem syscall layouts.
  Boot services (`kernel/src/lib.rs` `boot_init_dispatch`) pass a raw pointer, length, writable flag, and continuation in `arguments[0,1,2,4]`. Runtime (`crates/runtime/src/fs.rs`, `kernel/src/runtime.rs`) uses a `SharedBuffer` descriptor in `arguments[0..3]`. Same `SynFsList` opcode, different marshalling. Add a decode helper and a LIST pagination helper that seeds the path prefix so callers cannot forget the leftover-buffer protocol.

- [x] Replace `assert!` / `expect` in `write_service_image` with `fatal_kernel_halt`.
  `kernel/src/arch/x86_64.rs` panics if the Ring 3 image exceeds `SERVICE_CODE_PAGE_COUNT` pages or if entropy is missing. Other boot failures already use `fatal_kernel_halt(Status::…)`. Keep the 19-page / `.stack_guard` at `0x8000012ff8` layout, but fail through the same halt path.

- [x] Stop special-casing shell success status to raw `0`.
  `kernel/src/syscall.rs` `ghostos_call_gate_dispatch` rewrites caller 9 success to `status: 0` even if `Status::NORMAL.raw()` is not zero. C services compare `status != 0`. If the status encoding changes, the shell will treat success as failure or the reverse. Return the same `Status` the rest of the kernel uses.

- [x] Probe or enlarge the Ring 3 service stack before adding more shell locals.
  `SERVICE_PAGE_COUNT` is 19 code pages plus 8 stack pages (32 KiB). `_start` in `shell.c` keeps `line[512]` and `buffer[4096]` for the process lifetime; `execute_line` and account helpers add many 256-byte arrays. ELF apps get 1 MiB (`crates/app/src/loader.rs`). There is no stack probe. Either raise the boot-service stack or move the LIST buffer off the C stack.

- [x] Extract one virtio queue implementation.
  `virtual_machine/src/devices/virtio.rs` and `virtual_machine/src/devices/net/virtio.rs` both implement 0.9 queues, descriptor walks, and PCI register layout. Share the ring code so blk/console/rng and virtio-net cannot diverge.

- [x] Add `--no-passkey-web` (or a serial-only mode) to `start-ghostos.sh`.
  The VM defaults `passkey_web = true`. `virtual_machine/src/devices/serial.rs` strips enroll/login OSC markers and can hide prompts behind a spinner. Headless or scripted login then waits for a browser that never appears. Document the flag next to `docs/first-boot.md`.

- [x] Collapse workspace `members` and `default-members` to one source.
  Root `Cargo.toml` repeats ~66 crate paths. `default-members` omits `boot/uefi`, `tools/*`, and `examples/*`, so plain `cargo test` skips them. Generate both lists or use workspace defaults so they cannot drift.

- [x] Merge or clearly split tiny and overlapping crates.
  Single-file crates (`admission`, `api-compat`, `ghostos-kvd`, `numa`, `path-pattern`, `policy`, `protocol`, `service-scale`) each add a test target and a lock node. `ghostos-kvd` overlaps `crates/rms` embedded KV. Three script engines (`ghostos-script`, `ghostos-embedded-script`, `ghostos-wasm-script`) could share one facade with features.

- [x] Rename `tests/coverage_59_*.rs` to behavior names.
  Dozens of integration files encode old roadmap section numbers, not what they test. They are hard to map to `docs/test-inventory.toml`. Keep the inventory IDs in comments if needed.

- [x] Stop committing generated VM inventory and soak reports.
  `virtual_machine/tests/generated-inventory.toml` is generated. Soak JSON under `kernel/build/soak` and `virtual_machine/build/soak` is evidence, not source. Regenerate inventory in validation; put reports in `GHOSTOS_EVIDENCE_DIR`.

- [x] Deduplicate README and the book.
  `README.md` is over 1000 lines and restates boot, build, Docker, and shell material from `book/`. Keep a short quickstart in README and link to the book for essays.

- [x] Dual-read remaining SynOS magics instead of mixing brands.
  Still present: `SYNOSDSK` (`virtual_machine/src/devices/storage/system_disk.rs`), `SYNOSIG1` (snapshots), `.synos.lock`, AHCI serial `SYNOSVM00001`, confidential labels `synos-kem` / `synos-ss`. Compatibility is documented in `docs/persistence-compatibility.md`. Either keep them as frozen aliases with a converter, or bump format versions in CHANGELOG with dual-read.

- [x] Drop or regenerate the leftover `virtual_machine/Cargo.lock`.
  The VM is a workspace member (`edition = "2024"`) but still has a standalone lock and `edition = "2021"` in places. `scripts/build-and-test.sh` still `cd virtual_machine && cargo build --locked --release`. Build only from the workspace root.

- [x] Include `ghostos-netd` in mutation testing.
  `scripts/mutation.sh` covers fsd, status, auth, ghostfs, http, and the VM. Netd sits on the guest network boundary and is skipped.

---

## Speed

- [x] Index GhostFS directories instead of scanning every record per LIST.
  `crates/ghostfs/src/lib.rs` `list_directory_at` walks all records for each page. Each emitted entry calls `link_count_at`, which walks all records again. Large `/data` or `/packages` trees make `dir` quadratic. Keep a per-directory child list or cache link counts on the object.

- [x] Do not rescan the whole volume for every wildcard page.
  `expand_paths_page` restarts from ordinal 0 on each continuation (`crates/ghostfs/src/lib.rs`, used by `crates/fsd/src/daemon.rs` `write_wildcard_listing`). Resume from a stable cursor.

- [x] Stop size-optimizing the entire kernel.
  `[profile.release.package.ghostos-kernel] opt-level = "z"` plus workspace `lto = true` and `codegen-units = 1` shrinks `kernel.bin` but slows syscalls, interrupt dispatch, and every `start-ghostos.sh` rebuild. Keep `z` for cold paths or the image blob; compile `syscall.rs` / `arch/x86_64.rs` at `2` or `3`. Consider `lto = "thin"` for host tools. Stable Cargo cannot set per-file opt-level, so the kernel package is `opt-level = 2` and workspace release LTO is `thin`.

- [x] Allow parallel and incremental host builds.
  `.cargo/config.toml` sets `CARGO_BUILD_JOBS = 1` and `CARGO_INCREMENTAL = 0` (force false, but defaults kill iteration). That is for reproducible images. Scope those env vars to `scripts/build-bios-image.sh` / `scripts/check-reproducible-image.sh`, not every `cargo test`.

- [x] Use `panic = "unwind"` on host crates in dev.
  Workspace `[profile.dev] panic = "abort"` applies to the VM and all host tests. Kernel/uefi still need abort. Split profiles so host backtraces work while `ghostos-kernel` stays abort.

- [x] Cut redundant test sweeps.
  `scripts/test-all.sh` runs `cargo test`, then `cargo test --workspace --all-targets`, then the workspace command again under a recovery tier. `scripts/coverage.sh` runs workspace `llvm-cov` and then per-package coverage. One deterministic host pass plus an explicit `--workspace` gate is enough.

- [x] Add a smoke default so `cargo test` is not 70 crates.
  Default members currently include kernel + VM + almost every crate. A `ghostos-smoke` alias (abi, status, ghostfs, fsd, runtime, a thin VM test) would make local iteration match how `dir` actually gets fixed.

- [x] Avoid rewriting the LIST path on every continuation page.
  `print_directory` memset+copy of `/` on each page is required by the current in-band path protocol. A split path/output buffer removes that copy and the leftover-record class of bugs at the same time as the LIST ABI fix above.

---
## Central identity and VM fleet

- [ ] Define a central GhostOS directory contract for users, stable identity IDs, groups, roles, public credentials, credential labels, and revocation state. Keep private passkey material outside GhostOS.

- [ ] Add authenticated VM enrollment with a node identity, directory trust anchor, and explicit join/revoke lifecycle. A VM must not join by copying another VM's authorization database.

- [ ] Replace per-VM first-boot passkey creation with directory enrollment and login using a stable WebAuthn RP ID and origin. One user passkey should authenticate to every authorized VM without sharing private keys.

- [ ] Add a signed, short-lived directory authentication token bound to the user, credential, directory, target VM, challenge, expiry, and policy generation. Reject replay, wrong audience, stale generation, and revoked credentials.

- [ ] Map verified directory identity, groups, and roles to node-local kernel capabilities. The directory may authenticate and authorize identity claims; it must not directly mint unrestricted filesystem or kernel capabilities.

- [ ] Add bounded offline directory-cache behavior with revocation epochs, expiry, network-partition handling, and a separate local break-glass credential for recovery.

- [ ] Migrate the local `/system/security/authorization` bootstrap flow to coexist with central identity. Define first boot, directory unavailable, directory recovery, credential rotation, and last-local-admin behavior.

- [ ] Add VM-fleet integration coverage for one passkey logging into multiple VMs, node removal, credential revocation, offline login expiry, replay rejection, and private-key non-persistence.
