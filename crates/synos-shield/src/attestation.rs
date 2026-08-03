use synos_fabric::NodeId;
use synos_observability::{EventField, Level, audit_event, field};
use synos_status::IntoStatus;

use crate::{DIGEST_BYTES, Error, constant_time_equal, hmac_sha256};

const QUOTE_BYTES: usize = 4 + 1 + 8 + DIGEST_BYTES * 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum HardwareRoot {
    Tpm2 = 1,
    TrustZone = 2,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AttestationKey([u8; DIGEST_BYTES]);

impl AttestationKey {
    /// The key must be backed by a sealed TPM or TrustZone key slot.
    pub const fn new(bytes: [u8; DIGEST_BYTES]) -> Self {
        Self(bytes)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttestationQuote {
    pub node: NodeId,
    pub root: HardwareRoot,
    pub issued_at_us: u64,
    pub nonce: [u8; DIGEST_BYTES],
    pub measurement: [u8; DIGEST_BYTES],
    pub signature: [u8; DIGEST_BYTES],
}

impl AttestationQuote {
    pub fn new(
        node: NodeId,
        root: HardwareRoot,
        issued_at_us: u64,
        nonce: [u8; DIGEST_BYTES],
        measurement: [u8; DIGEST_BYTES],
        key: AttestationKey,
    ) -> Result<Self, Error> {
        if nonce.iter().all(|byte| *byte == 0) || measurement.iter().all(|byte| *byte == 0) {
            return Err(Error::InvalidInput)
        }
        let mut quote = Self {
            node,
            root,
            issued_at_us,
            nonce,
            measurement,
            signature: [0; DIGEST_BYTES],
        };
        quote.signature = quote.sign(key);
        Ok(quote)
    }

    fn material(&self) -> [u8; QUOTE_BYTES] {
        let mut material = [0; QUOTE_BYTES];
        material[..4].copy_from_slice(&self.node.raw().to_be_bytes());
        material[4] = self.root as u8;
        material[5..13].copy_from_slice(&self.issued_at_us.to_be_bytes());
        material[13..45].copy_from_slice(&self.nonce);
        material[45..77].copy_from_slice(&self.measurement);
        material
    }

    fn sign(&self, key: AttestationKey) -> [u8; DIGEST_BYTES] {
        hmac_sha256(&key.0, &self.material())
    }

    fn verify(&self, key: AttestationKey) -> Result<(), Error> {
        if constant_time_equal(&self.sign(key), &self.signature) {
            Ok(())
        } else {
            Err(Error::SignatureMismatch)
        }
    }
}

#[derive(Clone, Copy)]
struct TrustedNode {
    node: NodeId,
    root: HardwareRoot,
    measurement: [u8; DIGEST_BYTES],
    key: AttestationKey,
    challenge: Option<Challenge>,
    admitted: bool,
}

#[derive(Clone, Copy)]
struct Challenge {
    nonce: [u8; DIGEST_BYTES],
    expires_at_us: u64,
}

pub struct AdmissionController<const NODES: usize = 32> {
    nodes: [Option<TrustedNode>; NODES],
}

impl<const NODES: usize> AdmissionController<NODES> {
    pub const fn new() -> Self {
        Self { nodes: [None; NODES] }
    }

    pub fn register(
        &mut self,
        node: NodeId,
        root: HardwareRoot,
        measurement: [u8; DIGEST_BYTES],
        key: AttestationKey,
    ) -> Result<(), Error> {
        if measurement.iter().all(|byte| *byte == 0) {
            return Err(Error::InvalidInput)
        }
        if self.nodes.iter().flatten().any(|entry| entry.node == node) {
            return Err(Error::Duplicate)
        }
        let slot = self
            .nodes
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(Error::Capacity)?;
        *slot = Some(TrustedNode {
            node,
            root,
            measurement,
            key,
            challenge: None,
            admitted: false,
        });
        Ok(())
    }

    pub fn issue_challenge(
        &mut self,
        node: NodeId,
        nonce: [u8; DIGEST_BYTES],
        expires_at_us: u64,
    ) -> Result<(), Error> {
        if nonce.iter().all(|byte| *byte == 0) || expires_at_us == 0 {
            return Err(Error::InvalidInput)
        }
        let entry = self
            .nodes
            .iter_mut()
            .flatten()
            .find(|entry| entry.node == node)
            .ok_or(Error::NotFound)?;
        entry.challenge = Some(Challenge { nonce, expires_at_us });
        entry.admitted = false;
        Ok(())
    }

    pub fn admit(&mut self, quote: AttestationQuote, now_us: u64) -> Result<(), Error> {
        let entry = self
            .nodes
            .iter_mut()
            .flatten()
            .find(|entry| entry.node == quote.node)
            .ok_or(Error::Unauthorized)?;
        let challenge = entry.challenge.ok_or(Error::Unauthorized)?;
        if now_us > challenge.expires_at_us || quote.issued_at_us > now_us {
            return Err(Error::Expired)
        }
        if quote.root != entry.root
            || quote.nonce != challenge.nonce
            || quote.measurement != entry.measurement
        {
            audit_event!(
                Level::Warn,
                EventField::unsigned(field::CALLER, quote.node.raw() as u64),
                EventField::status(Error::Unauthorized.status()),
            );
            return Err(Error::Unauthorized)
        }
        quote.verify(entry.key)?;
        entry.admitted = true;
        entry.challenge = None;
        Ok(())
    }

    pub fn is_admitted(&self, node: NodeId) -> bool {
        self.nodes
            .iter()
            .flatten()
            .any(|entry| entry.node == node && entry.admitted)
    }

    pub fn require_admitted(&self, node: NodeId) -> Result<(), Error> {
        if self.is_admitted(node) {
            Ok(())
        } else {
            Err(Error::Unauthorized)
        }
    }
}

impl<const NODES: usize> Default for AdmissionController<NODES> {
    fn default() -> Self {
        Self::new()
    }
}
