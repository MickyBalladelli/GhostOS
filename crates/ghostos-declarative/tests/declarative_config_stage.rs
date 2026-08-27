// Inventory: coverage_59_10.rs (legacy roadmap section 59).
use ghostos_declarative::{
    AdmissionPolicy, ConfigurationArea, ConfigurationDiff, ConfigurationRuntime, DiscoveryPolicy,
    ParseError, ReconfigureError, ReconfigureManager, SignedConfiguration, SystemSpec,
    TpmConfigurationEnforcer, TpmSigningKey, TransportKind,
};
use ghostos_fabric::NodeId;
use ghostos_status::Status;
use ghostos_ghostfs::SynFs;

const CONFIG_V1: &str = r#"
[system]
schema = 1
revision = 1

[[service]]
name = "init"
image = 0x44
kind = "system"
restart = "on-failure"

[network]
hostname = "ghostos"

[[network.interface]]
name = "eth0"
address = "10.0.0.2"
mtu = 1500

[[network.route]]
destination = "0.0.0.0/0"
gateway = "10.0.0.1"
interface = "eth0"

[[capability]]
service = "init"
resource = "SYS$BOOT"
kind = "file"
rights = ["read", "execute"]
required = true
"#;

fn config(revision: u64) -> SystemSpec {
    SystemSpec::parse(&CONFIG_V1.replace("revision = 1", &format!("revision = {revision}"))).unwrap()
}

struct Runtime {
    events: Vec<&'static str>,
    fail_health: bool,
}

impl ConfigurationRuntime for Runtime {
    type Error = ();

    fn stage(&mut self, _configuration: &SystemSpec) -> Result<(), Self::Error> { self.events.push("stage"); Ok(()) }
    fn clear_stage(&mut self) { self.events.push("clear"); }
    fn commit(&mut self) -> Result<(), Self::Error> { self.events.push("commit"); Ok(()) }
    fn rollback(&mut self) { self.events.push("rollback"); }
    fn health_check(&mut self, _configuration: &SystemSpec) -> Result<(), Status> {
        self.events.push("health");
        if self.fail_health { Err(Status::BUSY) } else { Ok(()) }
    }
}

#[test]
fn declarative_parser_is_canonical_and_rejects_schema_conflicts() {
    let first = config(1);
    let second = SystemSpec::parse(&CONFIG_V1.replace("revision = 1", "revision = 1\n# stable comment")).unwrap();
    assert_eq!(first.digest(), second.digest());
    assert_eq!(first.schema(), 1);
    assert_eq!(first.services().count(), 1);
    assert_eq!(first.capabilities().next().unwrap().rights, 5);
    assert_eq!(first.network().interfaces().count(), 1);
    assert_eq!(SystemSpec::parse("[system]\nschema = 99\nrevision = 1"), Err(ParseError::UnsupportedSchema));
    assert_eq!(SystemSpec::parse("[system]\nschema = 1\nrevision = 1\nunknown = true"), Err(ParseError::UnknownKey));
}

#[test]
fn signed_activation_is_targeted_atomic_and_rollback_safe() {
    let key = TpmSigningKey::new([7; 32]);
    let mut enforcer = TpmConfigurationEnforcer::<2>::new();
    enforcer.trust_key(key).unwrap();
    let local = NodeId::LOCAL;
    let signed = SignedConfiguration::new(config(1), key, Some(local));
    enforcer.verify(&signed, local).unwrap();
    assert_eq!(enforcer.verify(&signed, NodeId::new(2).unwrap()), Err(ghostos_declarative::SignatureError::TargetMismatch));
    let unknown = SignedConfiguration::new(config(1), TpmSigningKey::new([8; 32]), None);
    assert_eq!(enforcer.verify(&unknown, local), Err(ghostos_declarative::SignatureError::UnknownKey));

    let mut filesystem = SynFs::<64>::new();
    let mut manager = ReconfigureManager::<2>::new();
    let mut runtime = Runtime { events: Vec::new(), fail_health: false };
    let first = manager.activate(&mut filesystem, &enforcer, &signed, local, &mut runtime).unwrap();
    assert_eq!(first.revision, 1);
    let signed_second = SignedConfiguration::new(config(2), key, Some(local)).expect_previous_revision(1);
    let second = manager.activate(&mut filesystem, &enforcer, &signed_second, local, &mut runtime).unwrap();
    assert_eq!(second.previous_revision, Some(1));
    assert_eq!(manager.active().unwrap().revision(), 2);
    let rollback = manager.rollback_last(&mut filesystem, &mut runtime).unwrap();
    assert_eq!(rollback.revision, 1);
    assert_eq!(manager.active().unwrap().revision(), 1);

    let failing = SignedConfiguration::new(config(3), key, Some(local)).expect_previous_revision(1);
    runtime.fail_health = true;
    assert_eq!(manager.activate(&mut filesystem, &enforcer, &failing, local, &mut runtime), Err(ReconfigureError::HealthCheck(Status::BUSY)));
    assert_eq!(manager.active().unwrap().revision(), 1);
    assert!(runtime.events.ends_with(&["health", "rollback", "clear"]));
}

#[test]
fn cluster_configuration_is_validated_diffed_and_overridden() {
    let source = r#"
[system]
schema = 1
revision = 4

[cluster]
id = 0x1234
name = "prod"
discovery = "hybrid"
membership = "automatic"
admission = "attested"
heartbeat_period_us = 500000
missed_heartbeat_limit = 3

[cluster.quorum]
voting_members = 3
required_votes = 2

[cluster.security]
require_attestation = true
trust_root = 0x55

[cluster.federation]
enabled = true
allow_remote_workloads = true
lease_ttl_us = 60000000
max_leases = 8

[[cluster.transport]]
name = "lan"
kind = "ethernet"
endpoint = "10.0.0.1:7000"
priority = 10

[[cluster.node-override]]
node = 2
heartbeat_period_us = 100000
transport = "ethernet"
"#;
    let configuration = SystemSpec::parse(source).unwrap();
    assert_eq!(configuration.cluster().identity.id, 0x1234);
    assert_eq!(configuration.cluster().discovery, DiscoveryPolicy::Hybrid);
    assert_eq!(configuration.cluster().admission, AdmissionPolicy::Attested);
    assert_eq!(configuration.cluster().quorum.required_votes, 2);
    assert_eq!(configuration.cluster().transports().count(), 1);
    assert_eq!(configuration.cluster().transports().next().unwrap().kind, TransportKind::Ethernet);
    assert_eq!(configuration.cluster().effective_for(ghostos_fabric::NodeId::new(2).unwrap()).heartbeat_period_us, 100000);
    configuration.validate().unwrap();

    let next = SystemSpec::parse(&source.replace("revision = 4", "revision = 5").replace("required_votes = 2", "required_votes = 3")).unwrap();
    let diff = ConfigurationDiff::between(Some(&configuration), &next).unwrap();
    assert!(diff.changes().any(|change| change.area == ConfigurationArea::Quorum));
    assert_eq!(diff.affected_nodes().collect::<Vec<_>>(), vec![2]);
}
