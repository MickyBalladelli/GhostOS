use core::fmt;

use ghostos_fabric::NodeId;
use ghostos_kernel::Rights;

use crate::token::CapabilityKey;

/// The complete scope checked for every privileged lease use.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LeaseContext {
    pub subject: NodeId,
    pub audience: NodeId,
    pub object: u64,
    pub tenant: u64,
    pub generation: u64,
    pub purpose: u64,
    pub required: Rights,
    pub now_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeaseError {
    Invalid,
    InvalidSignature,
    SubjectMismatch,
    AudienceMismatch,
    ObjectMismatch,
    TenantMismatch,
    GenerationMismatch,
    PurposeMismatch,
    RightsDenied,
    NotYetValid,
    Expired,
    Revoked,
    Replay,
    ReplayCapacity,
}

/// A signed, bounded capability lease. Every authorization input is bound to
/// the same signature, so a valid token for one daemon, object, tenant,
/// generation, or purpose cannot become valid in another context.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct CapabilityLease {
    pub issuer: NodeId,
    pub subject: NodeId,
    pub audience: NodeId,
    pub object: u64,
    pub tenant: u64,
    pub generation: u64,
    pub purpose: u64,
    pub rights: Rights,
    pub not_before_us: u64,
    pub expires_at_us: u64,
    pub revocation_epoch: u64,
    pub nonce: u64,
    tag: [u8; 32],
}

impl fmt::Debug for CapabilityLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapabilityLease")
            .field("value", &"[redacted]")
            .finish()
    }
}

impl CapabilityLease {
    pub const WIRE_BYTES: usize = 128;

    #[allow(clippy::too_many_arguments)]
    pub fn issue(
        key: CapabilityKey,
        issuer: NodeId,
        subject: NodeId,
        audience: NodeId,
        object: u64,
        tenant: u64,
        generation: u64,
        purpose: u64,
        rights: Rights,
        not_before_us: u64,
        expires_at_us: u64,
        revocation_epoch: u64,
        nonce: u64,
    ) -> Result<Self, LeaseError> {
        if object == 0
            || tenant == 0
            || generation == 0
            || purpose == 0
            || rights.is_empty()
            || expires_at_us <= not_before_us
            || revocation_epoch == 0
            || nonce == 0
        {
            return Err(LeaseError::Invalid)
        }
        let mut lease = Self {
            issuer,
            subject,
            audience,
            object,
            tenant,
            generation,
            purpose,
            rights,
            not_before_us,
            expires_at_us,
            revocation_epoch,
            nonce,
            tag: [0; 32],
        };
        lease.tag = key.authenticate(&lease.signing_bytes()).map_err(|_| LeaseError::Invalid)?;
        Ok(lease)
    }

    pub fn authorize(
        &self,
        key: CapabilityKey,
        context: LeaseContext,
        current_epoch: u64,
    ) -> Result<(), LeaseError> {
        if self.subject != context.subject {
            return Err(LeaseError::SubjectMismatch)
        }
        if self.audience != context.audience {
            return Err(LeaseError::AudienceMismatch)
        }
        if self.object != context.object {
            return Err(LeaseError::ObjectMismatch)
        }
        if self.tenant != context.tenant {
            return Err(LeaseError::TenantMismatch)
        }
        if self.generation != context.generation {
            return Err(LeaseError::GenerationMismatch)
        }
        if self.purpose != context.purpose {
            return Err(LeaseError::PurposeMismatch)
        }
        if !self.rights.contains(context.required) {
            return Err(LeaseError::RightsDenied)
        }
        if context.now_us < self.not_before_us {
            return Err(LeaseError::NotYetValid)
        }
        if context.now_us >= self.expires_at_us {
            return Err(LeaseError::Expired)
        }
        if self.revocation_epoch != current_epoch {
            return Err(LeaseError::Revoked)
        }
        let expected = key
            .authenticate(&self.signing_bytes())
            .map_err(|_| LeaseError::Invalid)?;
        if expected != self.tag {
            return Err(LeaseError::InvalidSignature)
        }
        Ok(())
    }

    pub fn encode(self) -> [u8; Self::WIRE_BYTES] {
        let mut output = [0; Self::WIRE_BYTES];
        output[0..4].copy_from_slice(b"SYLS");
        output[4] = 1;
        output[8..12].copy_from_slice(&self.issuer.raw().to_be_bytes());
        output[12..16].copy_from_slice(&self.subject.raw().to_be_bytes());
        output[16..20].copy_from_slice(&self.audience.raw().to_be_bytes());
        output[24..32].copy_from_slice(&self.object.to_be_bytes());
        output[32..40].copy_from_slice(&self.tenant.to_be_bytes());
        output[40..48].copy_from_slice(&self.generation.to_be_bytes());
        output[48..56].copy_from_slice(&self.purpose.to_be_bytes());
        output[56..58].copy_from_slice(&self.rights.bits().to_be_bytes());
        output[64..72].copy_from_slice(&self.not_before_us.to_be_bytes());
        output[72..80].copy_from_slice(&self.expires_at_us.to_be_bytes());
        output[80..88].copy_from_slice(&self.revocation_epoch.to_be_bytes());
        output[88..96].copy_from_slice(&self.nonce.to_be_bytes());
        output[96..128].copy_from_slice(&self.tag);
        output
    }

    pub fn decode(input: [u8; Self::WIRE_BYTES]) -> Result<Self, LeaseError> {
        if &input[0..4] != b"SYLS" || input[4] != 1 {
            return Err(LeaseError::Invalid)
        }
        let lease = Self {
            issuer: NodeId::new(read_u32(&input, 8)).ok_or(LeaseError::Invalid)?,
            subject: NodeId::new(read_u32(&input, 12)).ok_or(LeaseError::Invalid)?,
            audience: NodeId::new(read_u32(&input, 16)).ok_or(LeaseError::Invalid)?,
            object: read_u64(&input, 24),
            tenant: read_u64(&input, 32),
            generation: read_u64(&input, 40),
            purpose: read_u64(&input, 48),
            rights: Rights::from_bits(read_u16(&input, 56)).ok_or(LeaseError::Invalid)?,
            not_before_us: read_u64(&input, 64),
            expires_at_us: read_u64(&input, 72),
            revocation_epoch: read_u64(&input, 80),
            nonce: read_u64(&input, 88),
            tag: input[96..128].try_into().map_err(|_| LeaseError::Invalid)?,
        };
        if lease.object == 0
            || lease.tenant == 0
            || lease.generation == 0
            || lease.purpose == 0
            || lease.rights.is_empty()
            || lease.expires_at_us <= lease.not_before_us
            || lease.revocation_epoch == 0
            || lease.nonce == 0
        {
            return Err(LeaseError::Invalid)
        }
        Ok(lease)
    }

    fn signing_bytes(&self) -> [u8; 96] {
        let mut bytes = [0; 96];
        bytes[0..4].copy_from_slice(b"SYLS");
        bytes[4..8].copy_from_slice(&self.issuer.raw().to_be_bytes());
        bytes[8..12].copy_from_slice(&self.subject.raw().to_be_bytes());
        bytes[12..16].copy_from_slice(&self.audience.raw().to_be_bytes());
        bytes[16..24].copy_from_slice(&self.object.to_be_bytes());
        bytes[24..32].copy_from_slice(&self.tenant.to_be_bytes());
        bytes[32..40].copy_from_slice(&self.generation.to_be_bytes());
        bytes[40..48].copy_from_slice(&self.purpose.to_be_bytes());
        bytes[48..50].copy_from_slice(&self.rights.bits().to_be_bytes());
        bytes[56..64].copy_from_slice(&self.not_before_us.to_be_bytes());
        bytes[64..72].copy_from_slice(&self.expires_at_us.to_be_bytes());
        bytes[72..80].copy_from_slice(&self.revocation_epoch.to_be_bytes());
        bytes[80..88].copy_from_slice(&self.nonce.to_be_bytes());
        bytes
    }
}

/// Fixed-size replay guard for one-shot privileged operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LeaseReplayGuard<const CAPACITY: usize> {
    seen: [Option<u64>; CAPACITY],
}

impl<const CAPACITY: usize> LeaseReplayGuard<CAPACITY> {
    pub const fn new() -> Self {
        Self { seen: [None; CAPACITY] }
    }

    pub fn authorize_once(
        &mut self,
        lease: &CapabilityLease,
        key: CapabilityKey,
        context: LeaseContext,
        current_epoch: u64,
    ) -> Result<(), LeaseError> {
        lease.authorize(key, context, current_epoch)?;
        if self.seen.iter().flatten().any(|nonce| *nonce == lease.nonce) {
            return Err(LeaseError::Replay)
        }
        let slot = self
            .seen
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(LeaseError::ReplayCapacity)?;
        *slot = Some(lease.nonce);
        Ok(())
    }
}

impl<const CAPACITY: usize> Default for LeaseReplayGuard<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

fn read_u16(input: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([input[offset], input[offset + 1]])
}

fn read_u32(input: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        input[offset],
        input[offset + 1],
        input[offset + 2],
        input[offset + 3],
    ])
}

fn read_u64(input: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes([
        input[offset],
        input[offset + 1],
        input[offset + 2],
        input[offset + 3],
        input[offset + 4],
        input[offset + 5],
        input[offset + 6],
        input[offset + 7],
    ])
}
