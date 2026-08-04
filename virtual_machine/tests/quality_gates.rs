//! Deterministic checks for the VM quality-gate contract.

use std::fs;
use std::path::PathBuf;

fn inventory() -> String {
    fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/inventory.toml"))
        .expect("VM inventory must be readable")
}

#[test]
fn inventory_contract_names_all_quality_surfaces() {
    let text = inventory();
    for scenario in [
        "register_configuration",
        "normal_io",
        "reset",
        "interrupt",
        "malformed_input",
        "failure",
    ] {
        assert!(text.contains(scenario), "missing device scenario {scenario}");
    }
    for boot_path in ["name = \"bios\"", "name = \"uefi\"", "name = \"multiboot\""] {
        assert!(text.contains(boot_path), "missing boot path {boot_path}");
    }
    assert!(text.contains("serial_marker = \"SynOS kernel bootstrap\""));
}

#[test]
fn device_register_configuration_is_named() {
    assert!(inventory().contains("id = \"vm.device.register-configuration\""));
}

#[test]
fn device_normal_io_is_named() {
    assert!(inventory().contains("id = \"vm.device.normal-io\""));
}

#[test]
fn device_reset_is_named() {
    assert!(inventory().contains("id = \"vm.device.reset\""));
}

#[test]
fn device_interrupt_is_named() {
    assert!(inventory().contains("id = \"vm.device.interrupt\""));
}

#[test]
fn device_malformed_input_is_named() {
    assert!(inventory().contains("id = \"vm.device.malformed-input\""));
}

#[test]
fn device_failure_is_named() {
    assert!(inventory().contains("id = \"vm.device.failure\""));
}
