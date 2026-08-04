#![no_std]
#![forbid(unsafe_code)]

mod parser;
mod reconfigure;
mod signature;
mod cluster_config;

pub use cluster_config::{
    AdmissionPolicy, ClusterActivationReceipt, ClusterConfigurationManager, ClusterIdentitySpec,
    ClusterReconfigureError, ClusterSpec, ConfigurationArea, ConfigurationChange,
    ConfigurationDiff, ConfigurationQuorum, ConfigurationValidationError, DiffError,
    DiscoveryPolicy, EffectiveClusterSpec, FederationSpec, MembershipPolicy, NodeOverrideSpec,
    QuorumReceipt, QuorumSpec, ResourceSpec, SecuritySpec, TransportKind, TransportSpec,
    MAX_AFFECTED_NODES, MAX_AFFECTED_SERVICES, MAX_CLUSTER_DESCRIPTION_BYTES,
    MAX_CLUSTER_NAME_BYTES, MAX_CONFIGURATION_CHANGES, MAX_ENDPOINT_BYTES,
    MAX_NODE_OVERRIDES, MAX_TRANSPORT_NAME_BYTES, MAX_TRANSPORTS,
};

pub use parser::{
    BoundedText, CapabilityKind, CapabilityPolicy, CapabilityRights, MAX_ADDRESS_BYTES,
    MAX_CAPABILITY_POLICIES, MAX_NETWORK_INTERFACES, MAX_NETWORK_ROUTES, MAX_RESOURCE_NAME_BYTES,
    MAX_SERVICE_NAME_BYTES, MAX_SERVICES, NetworkInterface, NetworkRoute, NetworkSpec, ParseError,
    RestartPolicy, SYSTEM_SCHEMA_VERSION, ServiceKind, ServiceSpec, SystemSpec,
};
pub use reconfigure::{
    ActivationReceipt, ConfigurationRuntime, DEFAULT_RECONFIGURE_HISTORY, ReconfigureError,
    ReconfigureManager,
};
pub use signature::{
    ConfigurationSignature, KEY_ID_BYTES, SIGNATURE_BYTES, SignatureError, SignedConfiguration,
    TpmConfigurationEnforcer, TpmSigningKey,
};
