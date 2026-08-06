use synos_declarative::{AddressMode, ParseError, SystemSpec};

const CONFIG: &str = r#"
[system]
schema = 1
revision = 1

[network]
hostname = "synos"

[[network.interface]]
name = "eth0"
address = "10.0.0.2"
gateway = "10.0.0.1"
mtu = 1500

[[network.route]]
destination = "0.0.0.0/0"
gateway = "10.0.0.1"
interface = "eth0"
metric = 100
"#;

#[test]
fn network_mutations_create_new_revisions_and_preserve_routes() {
    let mut configuration = SystemSpec::parse(CONFIG).unwrap();
    configuration.set_network_hostname("node-1").unwrap();
    configuration
        .update_network_interface("eth0", Some("10.0.0.3"), None, Some(9000), Some(false), None)
        .unwrap();
    configuration
        .upsert_network_route("0.0.0.0/0", "10.0.0.254", "eth0", Some(50))
        .unwrap();

    assert_eq!(configuration.revision(), 4);
    assert_eq!(configuration.network().hostname.unwrap().as_str(), "node-1");
    let interface = configuration.network().interfaces().next().unwrap();
    assert_eq!(interface.address.as_str(), "10.0.0.3");
    assert_eq!(interface.mtu, 9000);
    assert!(!interface.enabled);
    assert_eq!(interface.mode, AddressMode::Static);
    let route = configuration.network().routes().next().unwrap();
    assert_eq!(route.gateway.as_str(), "10.0.0.254");
    assert_eq!(route.metric, 50);
}

#[test]
fn invalid_network_mutations_are_atomic() {
    let mut configuration = SystemSpec::parse(CONFIG).unwrap();
    let before = *configuration.network();
    assert_eq!(
        configuration.update_network_interface("missing", None, None, Some(1500), None, None),
        Err(ParseError::InvalidValue)
    );
    assert_eq!(configuration.revision(), 1);
    assert_eq!(*configuration.network(), before);
    assert_eq!(
        configuration.upsert_network_route("10.0.0.0/24", "10.0.0.1", "missing", None),
        Err(ParseError::InvalidValue)
    );
    assert_eq!(configuration.revision(), 1);
}

#[test]
fn dhcp_mode_and_lease_apply_restore_static_atomically() {
    let mut configuration = SystemSpec::parse(CONFIG).unwrap();
    configuration
        .update_network_interface("eth0", None, None, None, None, Some(AddressMode::Dhcp))
        .unwrap();
    assert_eq!(
        configuration.network().interfaces().next().unwrap().mode,
        AddressMode::Dhcp
    );
    configuration
        .apply_network_dhcp_lease("eth0", "10.0.0.50", Some("10.0.0.1"))
        .unwrap();
    let leased = configuration.network().interfaces().next().unwrap();
    assert_eq!(leased.address.as_str(), "10.0.0.50");
    assert_eq!(leased.mode, AddressMode::Dhcp);
    configuration
        .restore_network_static_interface("eth0", "10.0.0.2", Some("10.0.0.1"))
        .unwrap();
    let restored = configuration.network().interfaces().next().unwrap();
    assert_eq!(restored.address.as_str(), "10.0.0.2");
    assert_eq!(restored.mode, AddressMode::Static);
}
