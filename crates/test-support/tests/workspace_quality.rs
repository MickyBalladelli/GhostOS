use std::fs;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn workspace_default_members_are_the_smoke_set() {
    let cargo = fs::read_to_string(workspace_root().join("Cargo.toml")).expect("Cargo.toml");
    let defaults = toml_string_list(&cargo, "default-members");
    let expected = [
        "crates/abi",
        "crates/status",
        "crates/ghostfs",
        "crates/fsd",
        "crates/runtime",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    assert_eq!(
        defaults, expected,
        "default-members must stay focused on the GhostFS smoke set"
    );
}

#[test]
fn start_ghostos_forwards_no_passkey_web() {
    let script = fs::read_to_string(workspace_root().join("start-ghostos.sh")).expect("launcher");
    assert!(script.contains("--no-passkey-web"));
    assert!(script.contains("NO_PASSKEY_WEB"));
    let first_boot = fs::read_to_string(workspace_root().join("docs/first-boot.md")).expect("docs");
    assert!(first_boot.contains("./start-ghostos.sh --no-passkey-web"));
}

#[test]
fn build_and_test_builds_vm_from_workspace() {
    let script =
        fs::read_to_string(workspace_root().join("scripts/build-and-test.sh")).expect("script");
    assert!(script.contains("cargo build -p ghostos-vm --release"));
    assert!(!script.contains("cd \"$project_root/virtual_machine\""));
}

#[test]
fn mutation_testing_includes_netd() {
    let script = fs::read_to_string(workspace_root().join("scripts/mutation.sh")).expect("script");
    assert!(script.contains("ghostos-netd"));
}

#[test]
fn generated_inventory_and_soak_reports_are_gitignored() {
    let gitignore = fs::read_to_string(workspace_root().join(".gitignore")).expect("gitignore");
    assert!(gitignore.contains("generated-inventory.toml"));
    assert!(gitignore.contains("kernel/build/soak/"));
    assert!(gitignore.contains("virtual_machine/build/soak/"));
    assert!(gitignore.contains("virtual_machine/Cargo.lock"));
}

#[test]
fn default_members_are_listed_in_workspace_members() {
    let cargo = fs::read_to_string(workspace_root().join("Cargo.toml")).expect("Cargo.toml");
    let members = toml_string_list(&cargo, "members");
    let defaults = toml_string_list(&cargo, "default-members");
    for member in &defaults {
        assert!(
            members.contains(member),
            "{member} is in default-members but not members"
        );
    }
}

#[test]
fn kernel_release_is_not_size_optimized() {
    let cargo = fs::read_to_string(workspace_root().join("Cargo.toml")).expect("Cargo.toml");
    let kernel = cargo
        .split("[profile.release.package.ghostos-kernel]")
        .nth(1)
        .expect("kernel release profile");
    assert!(
        !kernel.contains("opt-level = \"z\""),
        "the kernel package must not be size-optimized"
    );
    assert!(
        kernel.contains("opt-level = 2") || kernel.contains("opt-level = 3"),
        "compile the kernel at opt-level 2 or 3"
    );
    assert!(
        cargo.contains("lto = \"thin\""),
        "host release builds should use thin LTO"
    );
}

#[test]
fn host_dev_profile_unwinds() {
    let cargo = fs::read_to_string(workspace_root().join("Cargo.toml")).expect("Cargo.toml");
    assert!(cargo.contains("[profile.dev]\npanic = \"unwind\""));
}

#[test]
fn cargo_config_does_not_serialize_host_builds() {
    let config =
        fs::read_to_string(workspace_root().join(".cargo/config.toml")).expect("cargo config");
    assert!(!config.contains("CARGO_BUILD_JOBS"));
    assert!(!config.contains("CARGO_INCREMENTAL"));
    let env = fs::read_to_string(workspace_root().join("scripts/reproducible-env.sh"))
        .expect("reproducible env");
    assert!(env.contains("CARGO_BUILD_JOBS=1"));
    assert!(env.contains("CARGO_INCREMENTAL=0"));
}

#[test]
fn bios_image_copies_pinned_ring3_images() {
    let script = fs::read_to_string(workspace_root().join("scripts/build-bios-image.sh"))
        .expect("bios image script");
    assert!(!script.contains("ls -td"));
    assert!(script.contains("build/kernel-ring3"));
    let build = fs::read_to_string(workspace_root().join("kernel/build.rs")).expect("kernel build");
    assert!(build.contains("build/kernel-ring3"));
}

#[test]
fn login_and_service_use_the_shell_syscall_contract() {
    for relative in [
        "userspace/boot-services/login.c",
        "userspace/boot-services/service.c",
    ] {
        let source = fs::read_to_string(workspace_root().join(relative)).expect(relative);
        assert!(
            source.contains(": \"+m\"(request), \"+m\"(response)"),
            "{relative} must keep request/response as +m outputs"
        );
        assert!(
            source.contains("\"rax\", \"rcx\", \"rdx\", \"r8\", \"r9\", \"r10\", \"r11\""),
            "{relative} must clobber the syscall volatile registers"
        );
    }
}

#[test]
fn readme_is_a_short_quickstart() {
    let readme = fs::read_to_string(workspace_root().join("README.md")).expect("README");
    assert!(
        readme.len() < 8_000,
        "README should stay a short quickstart, not a second book"
    );
    assert!(readme.contains("book/README.md"));
    assert!(readme.contains("./start-ghostos.sh"));
}

#[test]
fn crate_catalog_splits_tiny_packages() {
    let catalog = fs::read_to_string(workspace_root().join("book/appendix-a-crate-catalog.md"))
        .expect("catalog");
    assert!(catalog.contains("Tiny crates kept split"));
    assert!(catalog.contains("`ghostos-kvd`"));
    assert!(catalog.contains("`ghostos-rms`"));
    assert!(catalog.contains("`ghostos-embedded-script`"));
    assert!(catalog.contains("`ghostos-wasm-script`"));
}

#[test]
fn coverage_59_test_files_are_renamed() {
    let tests = workspace_root().join("crates");
    let leftover: Vec<_> = walkdir_coverage(&tests);
    assert!(
        leftover.is_empty(),
        "coverage_59_*.rs files must be renamed: {leftover:?}"
    );
}

fn walkdir_coverage(root: &PathBuf) -> Vec<String> {
    let mut leftover = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("coverage_59_") && name.ends_with(".rs"))
            {
                leftover.push(path.display().to_string());
            }
        }
    }
    leftover
}

fn toml_string_list(text: &str, key: &str) -> Vec<String> {
    let header = format!("{key} = [");
    let start = text.find(&header).unwrap_or_else(|| panic!("missing {key}"));
    let rest = &text[start + header.len()..];
    let end = rest.find(']').unwrap_or_else(|| panic!("unterminated {key}"));
    rest[..end]
        .lines()
        .filter_map(|line| {
            let line = line.trim().trim_end_matches(',');
            if line.starts_with('"') && line.ends_with('"') {
                Some(line.trim_matches('"').to_string())
            } else {
                None
            }
        })
        .collect()
}
