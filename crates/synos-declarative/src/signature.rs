use synos_fabric::NodeId;
use synos_status::{IntoStatus, Status};
use synos_system_model::ContentId;

use crate::SystemSpec;

pub const KEY_ID_BYTES: usize = 16;
pub const SIGNATURE_BYTES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignatureError {
    Capacity,
    InvalidSignature,
    TargetMismatch,
    UnknownKey,
}

impl IntoStatus for SignatureError {
    fn status(self) -> Status {
        match self {
            Self::Capacity => Status::NO_SPACE,
            Self::TargetMismatch => Status::INVALID_ARGUMENT,
            Self::InvalidSignature | Self::UnknownKey => Status::ACCESS_DENIED,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigurationSignature([u8; SIGNATURE_BYTES]);

impl ConfigurationSignature {
    pub const fn from_bytes(bytes: [u8; SIGNATURE_BYTES]) -> Self {
        Self(bytes)
    }

    pub const fn bytes(self) -> [u8; SIGNATURE_BYTES] {
        self.0
    }
}

/// A configuration key is expected to be an opaque handle to a sealed TPM
/// key in production. The deterministic HMAC adapter keeps this crate usable
/// in boot tests while preserving the signed-update boundary.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct TpmSigningKey([u8; SIGNATURE_BYTES]);

impl TpmSigningKey {
    pub const fn new(bytes: [u8; SIGNATURE_BYTES]) -> Self {
        Self(bytes)
    }

    pub fn id(self) -> [u8; KEY_ID_BYTES] {
        let digest = ContentId::hash(&self.0);
        let mut id = [0; KEY_ID_BYTES];
        id.copy_from_slice(&digest.as_bytes()[..KEY_ID_BYTES]);
        id
    }

    pub fn sign(
        self,
        configuration: &SystemSpec,
        target: Option<NodeId>,
    ) -> ConfigurationSignature {
        ConfigurationSignature(hmac_sha256(
            &self.0,
            &signature_material(configuration, target),
        ))
    }
}

impl core::fmt::Debug for TpmSigningKey {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("TpmSigningKey(..)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SignedConfiguration {
    configuration: SystemSpec,
    signer: [u8; KEY_ID_BYTES],
    signature: ConfigurationSignature,
    target: Option<NodeId>,
    expected_previous_revision: Option<u64>,
}

impl SignedConfiguration {
    pub fn new(configuration: SystemSpec, key: TpmSigningKey, target: Option<NodeId>) -> Self {
        Self {
            configuration,
            signer: key.id(),
            signature: key.sign(&configuration, target),
            target,
            expected_previous_revision: None,
        }
    }

    pub const fn configuration(&self) -> &SystemSpec {
        &self.configuration
    }

    pub const fn revision(&self) -> u64 {
        self.configuration.revision()
    }

    pub const fn signer(&self) -> [u8; KEY_ID_BYTES] {
        self.signer
    }

    pub const fn signature(&self) -> ConfigurationSignature {
        self.signature
    }

    pub const fn target(&self) -> Option<NodeId> {
        self.target
    }

    pub const fn expected_previous_revision(&self) -> Option<u64> {
        self.expected_previous_revision
    }

    pub const fn expect_previous_revision(mut self, revision: u64) -> Self {
        self.expected_previous_revision = Some(revision);
        self
    }
}

pub struct TpmConfigurationEnforcer<const KEYS: usize = 8> {
    keys: [Option<TpmSigningKey>; KEYS],
}

impl<const KEYS: usize> TpmConfigurationEnforcer<KEYS> {
    pub const fn new() -> Self {
        Self { keys: [None; KEYS] }
    }

    pub fn trust_key(&mut self, key: TpmSigningKey) -> Result<[u8; KEY_ID_BYTES], SignatureError> {
        if self
            .keys
            .iter()
            .flatten()
            .any(|trusted| trusted.id() == key.id())
        {
            return Err(SignatureError::InvalidSignature);
        }
        let slot = self
            .keys
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(SignatureError::Capacity)?;
        *slot = Some(key);
        Ok(key.id())
    }

    pub fn verify(&self, update: &SignedConfiguration, node: NodeId) -> Result<(), SignatureError> {
        if update.target.is_some_and(|target| target != node) {
            return Err(SignatureError::TargetMismatch);
        }
        let key = self
            .keys
            .iter()
            .flatten()
            .find(|key| key.id() == update.signer)
            .ok_or(SignatureError::UnknownKey)?;
        if key.sign(&update.configuration, update.target) == update.signature {
            Ok(())
        } else {
            Err(SignatureError::InvalidSignature)
        }
    }

    pub fn verify_cluster(
        &self,
        update: &SignedConfiguration,
        nodes: &[NodeId],
    ) -> Result<(), SignatureError> {
        if nodes.is_empty() {
            return Err(SignatureError::TargetMismatch);
        }
        for node in nodes {
            self.verify(update, *node)?;
        }
        Ok(())
    }
}

impl<const KEYS: usize> Default for TpmConfigurationEnforcer<KEYS> {
    fn default() -> Self {
        Self::new()
    }
}

fn signature_material(configuration: &SystemSpec, target: Option<NodeId>) -> [u8; 44] {
    let digest = configuration.digest();
    let mut material = [0; 44];
    material[..32].copy_from_slice(digest.as_bytes());
    material[32..40].copy_from_slice(&configuration.revision().to_be_bytes());
    material[40..44].copy_from_slice(&target.map_or(0, NodeId::raw).to_be_bytes());
    material
}

fn hmac_sha256(key: &[u8; SIGNATURE_BYTES], message: &[u8]) -> [u8; SIGNATURE_BYTES] {
    let mut normalized = [0; 64];
    normalized[..key.len()].copy_from_slice(key);
    let mut inner = [0; 108];
    for (index, byte) in normalized.iter().enumerate() {
        inner[index] = byte ^ 0x36;
    }
    inner[64..].copy_from_slice(message);
    let inner_hash = ContentId::hash(&inner);
    let mut outer = [0; 96];
    for (index, byte) in normalized.iter().enumerate() {
        outer[index] = byte ^ 0x5c;
    }
    outer[64..].copy_from_slice(inner_hash.as_bytes());
    *ContentId::hash(&outer).as_bytes()
}
