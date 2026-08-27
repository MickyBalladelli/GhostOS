/// Commands the Ring 3 C boot shell (`userspace/boot-services/shell.c`) must
/// recognize. The kernel Ring 0 shell is a debugger and uses this crate's
/// fuller command set, including EDIT and cluster/network routes.
pub const BOOT_SHELL_COMMANDS: &[&str] = &[
    "HELP",
    "DIRECTORY",
    "DIR",
    "LS",
    "CD",
    "CHDIR",
    "PWD",
    "CREATE",
    "TYPE",
    "CAT",
    "MKDIR",
    "RMDIR",
    "RD",
    "DELETE",
    "DEL",
    "WHOAMI",
    "LOGIN",
    "LOGOUT",
    "CREDENTIAL",
    "ACCOUNT",
    "WATCHDOG",
    "SHUTDOWN",
];
