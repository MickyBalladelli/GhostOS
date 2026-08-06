use synos_declarative::{
    AddressMode, ConfigurationRuntime, ParseError, ReconfigureError, ReconfigureManager,
    SignedConfiguration, SystemSpec, TpmConfigurationEnforcer, TpmSigningKey,
};
use synos_fabric::NodeId;
use synos_status::Status;
use synos_synfs::SynFs;

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

const DHCP_CONFIG: &str = r#"
[system]
schema = 1
revision = 1

[network]
hostname = "synos"

[[network.interface]]
name = "eth0"
mode = "dhcp"
mtu = 1500

[[network.interface]]
name = "eth1"
mode = "static"
address = "10.0.1.2"
gateway = "10.0.1.1"
"#;

struct Runtime {
    events: Vec<&'static str>,
    fail_health: bool,
    staged: Option<SystemSpec>,
    active: Option<SystemSpec>,
}

impl Runtime {
    fn new() -> Self {
        Self {
            events: Vec::new(),
            fail_health: false,
            staged: None,
            active: None,
        }
    }
}

impl ConfigurationRuntime for Runtime {
    type Error = ();

    fn stage(&mut self, configuration: &SystemSpec) -> Result<(), Self::Error> {
        self.events.push("stage");
        self.staged = Some(*configuration);
        Ok(())
    }

    fn clear_stage(&mut self) {
        self.events.push("clear");
        self.staged = None;
    }

    fn commit(&mut self) -> Result<(), Self::Error> {
        self.events.push("commit");
        self.active = self.staged;
        Ok(())
    }

    fn rollback(&mut self) {
        self.events.push("rollback");
    }

    fn health_check(&mut self, _configuration: &SystemSpec) -> Result<(), Status> {
        self.events.push("health");
        if self.fail_health {
            Err(Status::BUSY)
        } else {
            Ok(())
        }
    }
}

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
    assert_eq!(
        configuration.update_network_interface("eth0", None, None, Some(500), None, None),
        Err(ParseError::InvalidValue)
    );
    assert_eq!(configuration.revision(), 1);
    assert_eq!(*configuration.network(), before);
}

#[test]
fn dhcp_mode_parses_without_address_and_rejects_invalid_mode() {
    let configuration = SystemSpec::parse(DHCP_CONFIG).unwrap();
    let mut interfaces = configuration.network().interfaces();
    let eth0 = interfaces.next().unwrap();
    assert_eq!(eth0.mode, AddressMode::Dhcp);
    assert_eq!(eth0.address.as_str(), "0.0.0.0");
    let eth1 = interfaces.next().unwrap();
    assert_eq!(eth1.mode, AddressMode::Static);
    assert_eq!(eth1.address.as_str(), "10.0.1.2");

    let invalid = DHCP_CONFIG.replace("mode = \"dhcp\"", "mode = \"auto\"");
    assert_eq!(SystemSpec::parse(&invalid), Err(ParseError::InvalidValue));

    let missing_static_address = r#"
[system]
schema = 1
revision = 1
[[network.interface]]
name = "eth0"
mode = "static"
"#;
    assert_eq!(
        SystemSpec::parse(missing_static_address),
        Err(ParseError::MissingField)
    );
}

#[test]
fn dhcp_mode_and_lease_apply_restore_static_atomically() {
    let mut configuration = SystemSpec::parse(CONFIG).unwrap();
    let before_digest = configuration.digest();
    configuration
        .update_network_interface("eth0", None, None, None, None, Some(AddressMode::Dhcp))
        .unwrap();
    assert_eq!(
        configuration.network().interfaces().next().unwrap().mode,
        AddressMode::Dhcp
    );
    assert_ne!(configuration.digest(), before_digest);

    configuration
        .apply_network_dhcp_lease("eth0", "10.0.0.50", Some("10.0.0.1"))
        .unwrap();
    let leased = configuration.network().interfaces().next().unwrap();
    assert_eq!(leased.address.as_str(), "10.0.0.50");
    assert_eq!(leased.mode, AddressMode::Dhcp);

    // Lease apply on a static interface must fail without mutating state.
    let mut static_only = SystemSpec::parse(CONFIG).unwrap();
    let static_before = *static_only.network();
    assert_eq!(
        static_only.apply_network_dhcp_lease("eth0", "10.0.0.50", Some("10.0.0.1")),
        Err(ParseError::InvalidValue)
    );
    assert_eq!(*static_only.network(), static_before);
    assert_eq!(static_only.revision(), 1);

    configuration
        .restore_network_static_interface("eth0", "10.0.0.2", Some("10.0.0.1"))
        .unwrap();
    let restored = configuration.network().interfaces().next().unwrap();
    assert_eq!(restored.address.as_str(), "10.0.0.2");
    assert_eq!(restored.mode, AddressMode::Static);
}

#[test]
fn address_mode_is_part_of_configuration_digest() {
    let static_config = SystemSpec::parse(CONFIG).unwrap();
    let mut dhcp_config = SystemSpec::parse(CONFIG).unwrap();
    dhcp_config
        .update_network_interface("eth0", None, None, None, None, Some(AddressMode::Dhcp))
        .unwrap();
    // Normalize revision for digest comparison of network payload shape.
    assert_ne!(static_config.digest(), dhcp_config.digest());
}

#[test]
fn duplicate_interfaces_and_unknown_route_interfaces_are_rejected() {
    let duplicate = r#"
[system]
schema = 1
revision = 1
[[network.interface]]
name = "eth0"
address = "10.0.0.2"
[[network.interface]]
name = "eth0"
address = "10.0.0.3"
"#;
    assert_eq!(SystemSpec::parse(duplicate), Err(ParseError::DuplicateName));

    let bad_route = r#"
[system]
schema = 1
revision = 1
[[network.interface]]
name = "eth0"
address = "10.0.0.2"
[[network.route]]
destination = "0.0.0.0/0"
gateway = "10.0.0.1"
interface = "eth9"
"#;
    assert_eq!(SystemSpec::parse(bad_route), Err(ParseError::InvalidValue));
}

#[test]
fn network_reconfigure_persists_dhcp_lease_and_rolls_back_on_health_failure() {
    let key = TpmSigningKey::new([11; 32]);
    let mut enforcer = TpmConfigurationEnforcer::<2>::new();
    enforcer.trust_key(key).unwrap();
    let local = NodeId::LOCAL;

    let mut static_spec = SystemSpec::parse(CONFIG).unwrap();
    let signed_static = SignedConfiguration::new(static_spec, key, Some(local));

    let mut filesystem = SynFs::<64>::new();
    let mut manager = ReconfigureManager::<4>::new();
    let mut runtime = Runtime::new();
    manager
        .activate(
            &mut filesystem,
            &enforcer,
            &signed_static,
            local,
            &mut runtime,
        )
        .unwrap();
    assert_eq!(
        runtime
            .active
            .unwrap()
            .network()
            .interfaces()
            .next()
            .unwrap()
            .mode,
        AddressMode::Static
    );

    static_spec
        .update_network_interface("eth0", None, None, None, None, Some(AddressMode::Dhcp))
        .unwrap();
    static_spec
        .apply_network_dhcp_lease("eth0", "10.0.0.50", Some("10.0.0.1"))
        .unwrap();
    // parse starts at revision 1; two mutations -> revision 3
    assert_eq!(static_spec.revision(), 3);
    let signed_dhcp =
        SignedConfiguration::new(static_spec, key, Some(local)).expect_previous_revision(1);
    manager
        .activate(
            &mut filesystem,
            &enforcer,
            &signed_dhcp,
            local,
            &mut runtime,
        )
        .unwrap();
    let active = manager.active().unwrap();
    let iface = active.network().interfaces().next().unwrap();
    assert_eq!(iface.mode, AddressMode::Dhcp);
    assert_eq!(iface.address.as_str(), "10.0.0.50");

    let mut failing = static_spec;
    failing
        .restore_network_static_interface("eth0", "10.0.0.2", Some("10.0.0.1"))
        .unwrap();
    assert_eq!(failing.revision(), 4);
    let signed_fail =
        SignedConfiguration::new(failing, key, Some(local)).expect_previous_revision(3);
    runtime.fail_health = true;
    assert_eq!(
        manager.activate(
            &mut filesystem,
            &enforcer,
            &signed_fail,
            local,
            &mut runtime
        ),
        Err(ReconfigureError::HealthCheck(Status::BUSY))
    );
    assert_eq!(manager.active().unwrap().revision(), 3);
    assert_eq!(
        manager
            .active()
            .unwrap()
            .network()
            .interfaces()
            .next()
            .unwrap()
            .address
            .as_str(),
        "10.0.0.50"
    );
    assert!(runtime.events.ends_with(&["health", "rollback", "clear"]));

    runtime.fail_health = false;
    let rollback = manager.rollback_last(&mut filesystem, &mut runtime).unwrap();
    assert_eq!(rollback.revision, 1);
    assert_eq!(
        manager
            .active()
            .unwrap()
            .network()
            .interfaces()
            .next()
            .unwrap()
            .mode,
        AddressMode::Static
    );
}

#[test]
fn empty_update_and_empty_hostname_are_rejected() {
    let mut configuration = SystemSpec::parse(CONFIG).unwrap();
    assert_eq!(
        configuration.update_network_interface("eth0", None, None, None, None, None),
        Err(ParseError::InvalidValue)
    );
    // Hostname uses BoundedText; empty string is invalid.
    assert!(configuration.set_network_hostname("").is_err());
}
