# Rename SynOS → GhostOS

This is the work list for renaming the project, product, crates, tools, and
user-visible strings from **SynOS** to **GhostOS**. It is based on a full-tree
scan of the workspace (Cargo members, kernel, VM, Apple client, boot, docs,
scripts, ABI, and on-disk magics).

Do not treat this as a mechanical find-replace. On-disk magics, lock suffixes,
Rust target triples, and RPC strings are compatibility surfaces. Change those
only with an explicit format/ABI bump and a converter or dual-read window.

Suggested identifier map (confirm before starting):

| Kind | From | To |
| --- | --- | --- |
| Product / docs | SynOS | GhostOS |
| Crate / package prefix | `synos-*` | `ghostos-*` |
| Rust module prefix | `synos_` | `ghostos_` |
| Env vars | `SYNOS_*` | `GHOSTOS_*` |
| CLI binaries | `synos-vm`, `synos-loader`, `cargo-synos` | `ghostos-vm`, `ghostos-loader`, `cargo-ghostos` |
| Custom targets | `x86_64-unknown-synos` | `x86_64-unknown-ghostos` |
| Shell prompt / hostname | `SYNOS::ROOT`, hostname `synos` | `GHOSTOS::ROOT`, hostname `ghostos` |
| Docker | `synos:latest`, `SYNOS_QEMU_ACCEL` | `ghostos:latest`, `GHOSTOS_QEMU_ACCEL` |

Open product names (decide in task 0, then apply everywhere):

- `syn-shell` / `syn-script` (no `synos-` prefix today)
- `SynFS` / crate `synos-synfs` / magics `SYNFS001`, `SYNMNT01`
- Banner text `SYNCHRONOUS NETWORK OPERATING SYSTEM`

---

## 0. Decisions and inventory

## 1. Workspace crate and package names

Almost every Cargo package is `synos-*` even when the directory is not. Rename
package `name`, path members, and `use synos_*` imports together.

- [ ] Rename workspace members whose **directories** start with `synos-`:
  `synos-backup`, `synos-storaged`, `synos-kvd`, `synos-inference`,
  `synos-agent-bridge`, `synos-agentd`, `synos-embedded-script`,
  `synos-wasm-script`, `synos-audit`, `synos-shield`, `synos-confidential`,
  `synos-update`, `synos-heal`, `synos-top`, `synos-inspect`, `synos-debug`,
  `synos-replay`, `synos-webterm`, `synos-remote-display`, `synos-mesh`,
  `synos-declarative`, `synos-rustd`.
- [ ] Rename crate **package names** that are `synos-*` while keeping or also
  renaming directories: `synos-kernel`, `synos-vm`, `synos-uefi` / bin
  `synos-loader`, `synos-abi`, `synos-boot-protocol`, `synos-runtime`,
  `synos-status`, `synos-protocol`, `synos-synfs`, `synos-fsd`, `synos-auth`,
  `synos-netd`, `synos-http`, `synos-init`, `synos-app`, `synos-actors`,
  `synos-ipc`, `synos-client-sdk`, `synos-test-support`, `synos-posix-compat`,
  and the rest of the workspace `Cargo.toml` members.
- [ ] Rename `syn-shell` → `ghostos-shell` and `syn-script` → `ghostos-script` if
  task 0 includes them.
- [ ] Update root `Cargo.toml` `members` / `default-members`, every path
  dependency, `.cargo/config.toml` aliases (`-p synos-kernel`, `-p synos-uefi`),
  `Cargo.lock`, and `virtual_machine/Cargo.lock` if still present.
- [ ] Rename `fuzz` package `synos-fuzz` and any `synos-vm-fuzz-*` temp names.

## 2. Binaries, tools, and scripts

- [ ] Rename `virtual_machine` binary `synos-vm` → `ghostos-vm`; update CLI help,
  error prefixes (`synos-vm:`), and `start-synos.sh` → `start-ghostos.sh`.
- [ ] Rename UEFI binary `synos-loader` → `ghostos-loader`; update
  `scripts/check-reproducible-image.sh` and image layout docs.
- [ ] Rename `tools/cargo-synos` → `tools/cargo-ghostos` (`cargo ghostos build …`).
- [ ] Rename `tools/synos-compiler` → `tools/ghostos-compiler`.
- [ ] Rename `scripts/install-synos.sh`, `scripts/recover-synos.sh`, and every
  `scripts/*.sh` that hard-codes `-p synos-*` or `./target/release/synos-vm`.
- [ ] Update `boot/grub/grub.cfg` (`/synos.img`, `synos_loop`, `synos_host`).
- [ ] Update `Dockerfile`, `docker-compose.yml` (`synos`, `synos-cluster`,
  `synos:latest`), and `scripts/docker-*.sh`.

## 3. Rust targets, PAL, and compiler

These names leak into user builds (`App.toml`, `cargo synos`, `cfg(target_os)`).

- [ ] Rename `targets/x86_64-unknown-synos.json` and
  `targets/aarch64-unknown-synos.json`; update `os` / llvm target strings inside.
- [ ] Rename `crates/runtime/src/sys/synos.rs` and `synos_runtime::sys::synos`.
- [ ] Replace `cfg(target_os = "synos")` (see `examples/compiler-acceptance`).
- [ ] Rename `SYNOS_TOOLCHAIN_ROOT`, `SYNOS_REGISTRY_ROOT`, `SYNOS_SOURCE_ROOT`,
  `SYNOS_BUILD_ROOT`, `SYNOS_TEMP_ROOT` in runtime PAL and compiler.
- [ ] Rename kernel embed env vars `SYNOS_SERVICE_IMAGE`, `SYNOS_LOGIN_IMAGE`,
  `SYNOS_SHELL_IMAGE` (`kernel/src/arch/x86_64.rs`).
- [ ] Update `examples/hello-world/App.toml` target triple.

## 4. Kernel, boot, and shell branding

- [ ] Change bootstrap string `SynOS kernel bootstrap` and shell banner
  `SYNCHRONOUS NETWORK OPERATING SYSTEM`.
- [ ] Change prompt brand (`SYNOS::ROOT`) and default hostname `synos`.
- [ ] Rename `boot_synos_init` / service names such as `"synos-init"`.
- [ ] Update `kernel/src/physical_storage.rs` “no mountable AHCI SynOS system
  volume” and any operator-facing panic/help text.
- [ ] Update BIOS/UEFI comments and boot-contract tests that mention SynOS
  (`crates/test-support/tests/boot_contracts.rs`,
  `virtual_machine/tests/firmware_boot_synos_10_4.rs` — rename the test file).

## 5. Compatibility surfaces (do not blindly replace)

Bump format version and dual-read, or keep old magics and only change docs.

- [ ] System disk header `SYNOSDSK` (`virtual_machine/src/devices/storage/system_disk.rs`).
- [ ] Snapshot auth magic `SYNOSIG1` (`virtual_machine/src/snapshot.rs`).
- [ ] Migration HMAC domain `SYNOS-MIGRATION-HMAC-SHA256-V3`.
- [ ] Monitor HMAC domain `SYNOS-MONITOR-HMAC-SHA256-V1`.
- [ ] Disk lock suffix `.synos.lock` and recovery CLI text in README.
- [ ] Migration replay dir `.synos-vm-migration-replay`.
- [ ] Guest persistence ports/constants `SYNOS_PERSISTENCE_*`.
- [ ] ABI file `abi/synos-abi.toml` (filename + any SynOS strings; RPC magic
  `SYRP` is four bytes — changing it is a protocol break).
- [ ] Confidential crypto labels `synos-kem`, `synos-ss`, `synos-ctr`, `synos-tag`.
- [ ] Profile prefix `synos-profile-host-v1`.
- [ ] POSIX header `crates/posix-compat/include/synos_posix.h`.
- [ ] Inference proto `crates/synos-inference/proto/synos_inference.proto`.
- [ ] Kernel shell store magic `SYNFS001` if SynFS is renamed.
- [ ] Device serial strings `SYNOSVM00001` (AHCI/NVMe models).
- [ ] Add `CHANGELOG.md` entries under Disk formats / Snapshot and migration /
  Guest-visible for every wire change (`scripts/validate-changelog.py`).

## 6. Apple client

- [ ] Rename Swift package `SynOSControl` and products `SynOSClient`,
  `SynOSControlUI`, `SynOSControl`.
- [ ] Rename source trees
  `clients/apple/Sources/SynOSClient`,
  `SynOSControlUI`, `SynOSControlApp`.
- [ ] Update `clients/apple/Package.swift`, README, and any generated ABI
  (`GeneratedABI.swift`) after `abi/` rename.

## 7. Docs, book, and operator copy

Hundreds of hits live in `README.md`, `docs/`, `book/`, crate READMEs.

- [ ] Rewrite `README.md` title, bootstrap description, `start-ghostos.sh`, and
  disk-lock examples.
- [ ] Rename `book/01-what-synos-is.md` and retitle the book.
- [ ] Sweep `book/02-repository-map.md`, `appendix-a-crate-catalog.md`,
  `17-build-test-release.md`, `15-operations-and-lifecycle.md`.
- [ ] Sweep `docs/api.md`, `docs/testing.md`, `docs/native-compiler.md`,
  `docs/compatibility-matrix.md`, `docs/persistence-compatibility.md`,
  `docs/inventory-diagrams.md`, `docs/roadmap-metadata.toml`,
  `docs/test-inventory.toml`, `docs/test-coverage.toml`, `docs/invariants.toml`.
- [ ] Update `AGENTS.md` only if it mentions SynOS by name after the rename.
- [ ] Update golden logs `crates/test-support/golden/*` that contain `SynOS` /
  `SYNOS::`.

## 8. Tests, fuzz, CI, and env vars

- [ ] Replace `cargo test -p synos-*` in `scripts/test-all.sh`,
  `scripts/test-vm-matrix.sh`, `scripts/mutation.sh`, `scripts/coverage.sh`,
  `scripts/qemu-*.sh`, `scripts/full-validation.sh`.
- [ ] Rename env vars: `SYNOS_FULL_VALIDATION`, `SYNOS_RUN_QEMU_TESTS`,
  `SYNOS_QEMU_ACCEL`, `SYNOS_GUEST_MEMORY`, `SYNOS_CLUSTER_NODES`,
  `SYNOS_TEST_RUN_ID`, `SYNOS_EVIDENCE_DIR`, `SYNOS_VM_CPUS`,
  `SYNOS_BENCH_*`, `SYNOS_MUTATION_PACKAGE`, `SYNOS_LOCK_*`.
- [ ] Rename fuzz corpus temp prefixes (`synos-vm-fuzz-image-*`,
  `synos-vm-cluster-kernel-*`).
- [ ] Update coverage IDs / file names that embed `synos` only if they are not
  frozen evidence hashes; keep historical evidence filenames if they are
  immutable artifacts.

## 9. Verification

- [ ] `rg -i 'synos|syn-os|SYNOS|SynOS'` on the tree excluding `archive/` (and
  excluding kept magics if task 0 said keep them). Remaining hits should be
  documented compatibility aliases only.
- [ ] `cargo test` default workspace members.
- [ ] `./scripts/build-bios-image.sh` and boot to a GhostOS prompt.
- [ ] `cargo test -p ghostos-vm` (or new VM package name) including lock-recovery
  strings.
- [ ] Apple package `swift build` if the toolchain is present.
- [ ] Docker compose config still builds after image/env rename.

## 10. Out of tree / operator follow-up

- [ ] Rename local checkout directory and any git remotes/org names.
- [ ] Update Docker Hub / GHCR image names if published.
- [ ] Warn operators: old `.synos.lock` files, system disks, snapshots, and
  `x86_64-unknown-synos` toolchains will not match until converted.

---

# Project simplification opportunities

Suggestions from a full-tree review (803 files, 73 workspace members). Each item
is independent; do them one at a time with `cargo test` between steps.

## 1. Workspace and crate structure

- [ ] Merge the eight single-file crates — `admission`, `api-compat`,
  `ghostos-kvd`, `numa`, `path-pattern`, `policy`, `protocol`,
  `service-scale` (each is just `Cargo.toml` + one `lib.rs`) — into one
  support crate (for example `ghostos-support`) or into their primary
  consumers. Each removal drops one entry from `members` *and*
  `default-members`, one Cargo.lock node, and one test target.
- [ ] Stop duplicating the member list: root `Cargo.toml` repeats all ~68
  members verbatim in `default-members`. If default-members must stay,
  generate both lists from one source (build script or workspace inheritance)
  so they cannot drift.
- [ ] Consolidate the three scripting runtimes (`ghostos-script` native DCL,
  `ghostos-embedded-script` Rhai, `ghostos-wasm-script` Wasmi). Keep the
  engines, but consider one facade crate with cargo features instead of three
  parallel crates, READMEs, and test suites.
- [ ] Shrink the AI stack: `ghostos-agentd` is three files — fold it into
  `ghostos-agent-bridge` or `ghostos-inference`; review whether `llm-runtime`
  and `compute` can share their allocator/tensor plumbing.
- [ ] Pick one KV story: `ghostos-rms` already ships an embedded key-value
  database over GhostFS, and `ghostos-kvd` is a second KV layer on GhostFS.
  Merge or clearly split their responsibilities.
- [ ] Review the two virtio implementations in the VM
  (`virtual_machine/src/devices/virtio.rs`, 885 lines vs
  `virtual_machine/src/devices/net/virtio.rs`, 464 lines) and share the
  queue/ring code.
- [ ] Define one shell roadmap: `kernel/src/shell.rs` (Ring 0 console shell)
  and the `ghostos-shell` crate (Ring 3 DCL shell) duplicate parser/editor
  concepts. Long term, move console handling onto the Ring 3 shell.
- [ ] Slim the Apple client surface: `clients/apple` has three products
  (Client, ControlUI, ControlApp) where one library + one app binary may do.

## 2. Tests

- [ ] Rename the 42 roadmap-named test files (`tests/coverage_59_5.rs` …
  `coverage_59_10.rs`, `coverage_58_1.rs`) to descriptive names. The IDs tie
  test files to roadmap metadata, make navigation hard, and invite collisions
  when crates gain more tests.
- [ ] Where a crate has several single-test `tests/*.rs` files, group related
  coverage tests into one integration file per theme to cut compile time.
- [ ] `virtual_machine/tests/generated-inventory.toml` is generated yet
  tracked; regenerate it in CI/local validation instead of committing it.

## 3. Scripts and evidence machinery

- [ ] `scripts/` holds ~35 standalone Python validators/generators plus 27
  shell scripts. Extract the shared helpers (inventory loading, evidence
  paths, changelog access) into one Python package and expose a single CLI
  with subcommands, keeping thin wrappers only where names are load-bearing.
- [ ] Untracked build outputs are tracked: `kernel/build/soak/capabilities/
  report.json` and `virtual_machine/build/soak/lifecycle/report.json` are
  committed. Move evidence reports to an ignored evidence directory (the
  `GHOSTOS_EVIDENCE_DIR` convention) and reference them by path in manifests.
- [ ] Several docs are machine-checked registries (`docs/test-inventory.toml`,
  `docs/test-coverage.toml`, `docs/invariants.toml`,
  `docs/roadmap-metadata.toml`). Generate the derivable ones from source
  metadata in one step rather than hand-maintaining four files validated by
  four scripts.

## 4. Documentation

- [ ] Trim the 1050-line `README.md`: keep boot/build/test quickstart and
  links; the feature essays largely duplicate `book/` chapters and `docs/`.
- [ ] Fold `archive/TODO-*.md` into a single `archive/HISTORY.md` (or accept
  them as-is) so only one live TODO exists at the root.
- [ ] Crate-level READMEs repeat book content; keep one authoritative
  description per subsystem in `book/` and link to it.

## 5. Verification for every simplification step

- [ ] `cargo test` (default members) passes.
- [ ] `python3 scripts/validate-documentation.py` still passes after doc moves.
- [ ] No compatibility-surface change (magics, wire formats, target triples)
  is bundled into refactor commits.
