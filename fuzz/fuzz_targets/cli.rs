#![no_main]

use libfuzzer_sys::fuzz_target;
use syn_shell::{
    audit::register_audit_command,
    cluster::register_cluster_commands_without_help,
    diagnostics::register_builtin_commands,
    filesystem::register_filesystem_commands,
    firewall::register_firewall_commands,
    network::register_network_commands,
    parser::CommandRegistry,
    rust::register_rust_commands,
    storage::register_storage_commands,
};

fn registry() -> CommandRegistry<128> {
    let mut registry = CommandRegistry::new();
    let _ = register_audit_command(&mut registry);
    let _ = register_cluster_commands_without_help(&mut registry);
    let _ = register_builtin_commands(&mut registry);
    let _ = register_filesystem_commands(&mut registry);
    let _ = register_firewall_commands(&mut registry);
    let _ = register_network_commands(&mut registry);
    let _ = register_rust_commands(&mut registry);
    let _ = register_storage_commands(&mut registry);
    registry
}

fuzz_target!(|data: &[u8]| {
    let input = String::from_utf8_lossy(&data[..data.len().min(64 * 1024)]);
    let registry = registry();
    let _ = registry.parse(&input);
    let _ = registry.suggestions(&input);
    let _ = registry.unique_suggestion(&input);
});
