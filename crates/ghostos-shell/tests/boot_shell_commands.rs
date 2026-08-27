use ghostos_shell::boot_commands::BOOT_SHELL_COMMANDS;

#[test]
fn boot_shell_contains_shared_command_names() {
    let source = include_str!("../../../userspace/boot-services/shell.c");
    for name in BOOT_SHELL_COMMANDS {
        assert!(
            source.contains(name),
            "userspace/boot-services/shell.c must contain {name}"
        );
    }
}

#[test]
fn kernel_shell_is_documented_as_ring0_debugger() {
    let source = include_str!("../../../kernel/src/shell.rs");
    assert!(source.contains("Ring 0 operator/debugger shell"));
    assert!(source.contains("userspace/boot-services/shell.c"));
}
