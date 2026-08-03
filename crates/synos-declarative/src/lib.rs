#![no_std]
#![forbid(unsafe_code)]

mod parser;
mod reconfigure;
mod signature;

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
