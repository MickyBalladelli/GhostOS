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

#[test]
fn boot_shell_cd_uses_a_working_directory() {
    let source = include_str!("../../../userspace/boot-services/shell.c");
    assert!(source.contains("static const u64 WORKING_DIRECTORY = 0x0000008000013400ULL"));
    assert!(source.contains("char *directory = (char *)WORKING_DIRECTORY"));
    assert!(source.contains("execute_line(line, buffer, directory)"));
    assert!(source.contains("command_named(command, command_length, \"CD\", 2)"));
    assert!(source.contains("copy_cstr(directory, argument)"));
    assert!(source.contains("write_text(directory)"));
    assert!(
        !source.contains("write_text(\"/\\n\")"),
        "PWD must print the working directory, not a hardcoded root"
    );
}

#[test]
fn boot_shell_ls_copies_cwd_before_listing() {
    let source = include_str!("../../../userspace/boot-services/shell.c");
    let stack_start = 0x0000_0080_0001_3000u64;
    let cwd = 0x0000_0080_0001_3400u64;
    assert!(
        cwd >= stack_start + 1024,
        "cwd must sit after the role byte and resource manifest"
    );
    assert!(
        source.contains("copy_cstr(argument, directory)"),
        "empty LS must copy cwd into a local before print_directory"
    );
    assert!(
        !source.contains("print_directory(directory, buffer)"),
        "print_directory must not use the cwd slot adjacent to the LIST buffer"
    );
    assert!(
        !source.contains("char directory[256]"),
        "cwd must not live in _start next to the 4 KiB LIST buffer"
    );
}
