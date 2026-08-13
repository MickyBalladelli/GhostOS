use core::fmt;

use synos_fabric::{AddressRange, NodeId};
use synos_kernel::{FederationClusterId, Rights};

use crate::{
    LendingError, LendingKind, LendingRights, ResourceLender, RevocationAction,
    token::{
        CapabilityCaveat, CapabilityKey, CryptographicCapability, TokenError, TransportRights,
    },
};

pub const MAX_FEDERATED_PEERS: usize = 32;
pub const MAX_REVOCATION_LATENCY_US: u64 = 999;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ClusterId(u128);

impl ClusterId {
    pub const fn new(raw: u128) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u128 {
        self.0
    }

    pub const fn dlm_id(self) -> FederationClusterId {
        match FederationClusterId::new(self.0) {
            Some(id) => id,
            None => unreachable!(),
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct DiscoveryAnnouncement {
    pub cluster: ClusterId,
    pub gateway: NodeId,
    pub epoch: u64,
    pub issued_at_us: u64,
    pub expires_at_us: u64,
    pub nonce: u64,
    pub transports: TransportRights,
    authenticator: [u8; 32],
}

impl fmt::Debug for DiscoveryAnnouncement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DiscoveryAnnouncement")
            .field("value", &"[redacted]")
            .finish()
    }
}

impl DiscoveryAnnouncement {
    pub const WIRE_BYTES: usize = 96;

    pub fn issue(
        key: CapabilityKey,
        cluster: ClusterId,
        gateway: NodeId,
        epoch: u64,
        issued_at_us: u64,
        expires_at_us: u64,
        nonce: u64,
        transports: TransportRights,
    ) -> Result<Self, FederationError> {
        if epoch == 0
            || nonce == 0
            || expires_at_us <= issued_at_us
            || TransportRights::from_bits(transports.bits()).is_none()
        {
            return Err(FederationError::Invalid);
        }
        let mut announcement = Self {
            cluster,
            gateway,
            epoch,
            issued_at_us,
            expires_at_us,
            nonce,
            transports,
            authenticator: [0; 32],
        };
        announcement.authenticator = key
            .authenticate(&announcement.payload())
            .map_err(FederationError::Token)?;
        Ok(announcement)
    }

    pub fn verify(&self, key: CapabilityKey, now_us: u64) -> Result<(), FederationError> {
        if self.epoch == 0
            || self.nonce == 0
            || self.expires_at_us <= self.issued_at_us
            || TransportRights::from_bits(self.transports.bits()).is_none()
        {
            return Err(FederationError::Invalid);
        }
        if now_us < self.issued_at_us || now_us >= self.expires_at_us {
            return Err(FederationError::Expired);
        }
        key.verify_authenticator(&self.payload(), &self.authenticator)
            .map_err(FederationError::Token)
    }

    pub fn encode(self) -> [u8; Self::WIRE_BYTES] {
        let mut wire = [0; Self::WIRE_BYTES];
        wire[..60].copy_from_slice(&self.payload());
        wire[64..96].copy_from_slice(&self.authenticator);
        wire
    }

    pub fn decode(wire: [u8; Self::WIRE_BYTES]) -> Result<Self, FederationError> {
        if &wire[0..4] != b"SYFD" || wire[4] != 1 {
            return Err(FederationError::Invalid);
        }
        let mut authenticator = [0; 32];
        authenticator.copy_from_slice(&wire[64..96]);
        Ok(Self {
            cluster: ClusterId::new(read_u128(&wire, 8)).ok_or(FederationError::Invalid)?,
            gateway: NodeId::new(read_u32(&wire, 24)).ok_or(FederationError::Invalid)?,
            epoch: read_u64(&wire, 28),
            issued_at_us: read_u64(&wire, 36),
            expires_at_us: read_u64(&wire, 44),
            nonce: read_u64(&wire, 52),
            transports: TransportRights::from_bits(wire[5]).ok_or(FederationError::Invalid)?,
            authenticator,
        })
    }

    fn payload(&self) -> [u8; 60] {
        let mut payload = [0; 60];
        payload[0..4].copy_from_slice(b"SYFD");
        payload[4] = 1;
        payload[5] = self.transports.bits();
        payload[8..24].copy_from_slice(&self.cluster.raw().to_be_bytes());
        payload[24..28].copy_from_slice(&self.gateway.raw().to_be_bytes());
        payload[28..36].copy_from_slice(&self.epoch.to_be_bytes());
        payload[36..44].copy_from_slice(&self.issued_at_us.to_be_bytes());
        payload[44..52].copy_from_slice(&self.expires_at_us.to_be_bytes());
        payload[52..60].copy_from_slice(&self.nonce.to_be_bytes());
        payload
    }
}

#[derive(Clone, Copy)]
struct Peer {
    announcement: DiscoveryAnnouncement,
}

/// Fixed peer directory learned directly from authenticated cluster gateways.
pub struct PeerDirectory<const CAPACITY: usize = MAX_FEDERATED_PEERS> {
    peers: [Option<Peer>; CAPACITY],
}

impl<const CAPACITY: usize> PeerDirectory<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            peers: [None; CAPACITY],
        }
    }

    pub fn observe(
        &mut self,
        announcement: DiscoveryAnnouncement,
        peering_key: CapabilityKey,
        now_us: u64,
    ) -> Result<(), FederationError> {
        announcement.verify(peering_key, now_us)?;
        if let Some(peer) = self
            .peers
            .iter_mut()
            .flatten()
            .find(|peer| peer.announcement.cluster == announcement.cluster)
        {
            if announcement.epoch < peer.announcement.epoch
                || (announcement.epoch == peer.announcement.epoch
                    && announcement.nonce <= peer.announcement.nonce)
            {
                return Err(FederationError::Replay);
            }
            peer.announcement = announcement;
            return Ok(());
        }
        let slot = self
            .peers
            .iter_mut()
            .find(|peer| peer.is_none())
            .ok_or(FederationError::Capacity)?;
        *slot = Some(Peer { announcement });
        Ok(())
    }

    pub fn peer(
        &self,
        cluster: ClusterId,
        now_us: u64,
    ) -> Result<DiscoveryAnnouncement, FederationError> {
        let announcement = self
            .peers
            .iter()
            .flatten()
            .find(|peer| peer.announcement.cluster == cluster)
            .map(|peer| peer.announcement)
            .ok_or(FederationError::UnknownCluster)?;
        if now_us >= announcement.expires_at_us {
            Err(FederationError::Expired)
        } else {
            Ok(announcement)
        }
    }
}

impl<const CAPACITY: usize> Default for PeerDirectory<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FederatedResourceKind {
    Ram = 1,
    Vram = 2,
    Cpu = 3,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct FederatedResourceOffer {
    pub provider: ClusterId,
    pub resource: u64,
    pub kind: FederatedResourceKind,
    pub amount: u64,
    pub address: u64,
    pub rights: Rights,
    pub transports: TransportRights,
    pub epoch: u64,
    pub expires_at_us: u64,
    authenticator: [u8; 32],
}

impl fmt::Debug for FederatedResourceOffer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FederatedResourceOffer")
            .field("value", &"[redacted]")
            .finish()
    }
}

impl FederatedResourceOffer {
    pub const WIRE_BYTES: usize = 96;

    #[allow(clippy::too_many_arguments)]
    pub fn issue(
        key: CapabilityKey,
        provider: ClusterId,
        resource: u64,
        kind: FederatedResourceKind,
        amount: u64,
        address: u64,
        rights: Rights,
        transports: TransportRights,
        epoch: u64,
        expires_at_us: u64,
    ) -> Result<Self, FederationError> {
        if resource == 0
            || amount == 0
            || epoch == 0
            || expires_at_us == 0
            || rights.is_empty()
            || (kind == FederatedResourceKind::Cpu && address != 0)
            || (kind != FederatedResourceKind::Cpu && address.checked_add(amount).is_none())
        {
            return Err(FederationError::Invalid);
        }
        let expected = match kind {
            FederatedResourceKind::Cpu => Rights::EXECUTE,
            FederatedResourceKind::Ram | FederatedResourceKind::Vram => {
                Rights::READ.union(Rights::WRITE)
            }
        };
        if rights != expected {
            return Err(FederationError::Invalid);
        }
        let mut offer = Self {
            provider,
            resource,
            kind,
            amount,
            address,
            rights,
            transports,
            epoch,
            expires_at_us,
            authenticator: [0; 32],
        };
        offer.authenticator = key
            .authenticate(&offer.payload())
            .map_err(FederationError::Token)?;
        Ok(offer)
    }

    pub fn verify(&self, key: CapabilityKey, now_us: u64) -> Result<(), FederationError> {
        let expected_rights = match self.kind {
            FederatedResourceKind::Cpu => Rights::EXECUTE,
            FederatedResourceKind::Ram | FederatedResourceKind::Vram => {
                Rights::READ.union(Rights::WRITE)
            }
        };
        if self.resource == 0
            || self.amount == 0
            || self.epoch == 0
            || self.expires_at_us == 0
            || self.rights != expected_rights
            || TransportRights::from_bits(self.transports.bits()).is_none()
            || (self.kind == FederatedResourceKind::Cpu && self.address != 0)
            || (self.kind != FederatedResourceKind::Cpu
                && self.address.checked_add(self.amount).is_none())
        {
            return Err(FederationError::Invalid);
        }
        if now_us >= self.expires_at_us {
            return Err(FederationError::Expired);
        }
        key.verify_authenticator(&self.payload(), &self.authenticator)
            .map_err(FederationError::Token)
    }

    pub fn encode(self) -> [u8; Self::WIRE_BYTES] {
        let mut wire = [0; Self::WIRE_BYTES];
        wire[..64].copy_from_slice(&self.payload());
        wire[64..96].copy_from_slice(&self.authenticator);
        wire
    }

    pub fn decode(wire: [u8; Self::WIRE_BYTES]) -> Result<Self, FederationError> {
        if &wire[0..4] != b"SYFO" || wire[4] != 1 {
            return Err(FederationError::Invalid);
        }
        let kind = match wire[5] {
            1 => FederatedResourceKind::Ram,
            2 => FederatedResourceKind::Vram,
            3 => FederatedResourceKind::Cpu,
            _ => return Err(FederationError::Invalid),
        };
        let mut authenticator = [0; 32];
        authenticator.copy_from_slice(&wire[64..96]);
        Ok(Self {
            provider: ClusterId::new(read_u128(&wire, 8)).ok_or(FederationError::Invalid)?,
            resource: read_u64(&wire, 24),
            kind,
            amount: read_u64(&wire, 32),
            address: read_u64(&wire, 40),
            rights: Rights::from_bits(wire[7] as u16).ok_or(FederationError::Invalid)?,
            transports: TransportRights::from_bits(wire[6]).ok_or(FederationError::Invalid)?,
            expires_at_us: read_u64(&wire, 48),
            epoch: read_u64(&wire, 56),
            authenticator,
        })
    }

    fn payload(&self) -> [u8; 64] {
        let mut payload = [0; 64];
        payload[0..4].copy_from_slice(b"SYFO");
        payload[4] = 1;
        payload[5] = self.kind as u8;
        payload[6] = self.transports.bits();
        payload[7] = self.rights.bits() as u8;
        payload[8..24].copy_from_slice(&self.provider.raw().to_be_bytes());
        payload[24..32].copy_from_slice(&self.resource.to_be_bytes());
        payload[32..40].copy_from_slice(&self.amount.to_be_bytes());
        payload[40..48].copy_from_slice(&self.address.to_be_bytes());
        payload[48..56].copy_from_slice(&self.expires_at_us.to_be_bytes());
        payload[56..64].copy_from_slice(&self.epoch.to_be_bytes());
        payload
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FederatedLease {
    pub provider: ClusterId,
    pub federation_epoch: u64,
    pub capability: CryptographicCapability,
}

impl FederatedLease {
    /// Restrict a lease before handing it to a tenant or delegated worker.
    /// The issuer key is not needed because attenuation can only remove power.
    pub fn attenuate(self, caveat: CapabilityCaveat) -> Result<Self, FederationError> {
        Ok(Self {
            capability: self
                .capability
                .attenuate(caveat)
                .map_err(FederationError::Token)?,
            ..self
        })
    }
}

pub fn accept_offer<const PEERS: usize, const LOANS: usize>(
    directory: &PeerDirectory<PEERS>,
    peering_key: CapabilityKey,
    lender: &mut ResourceLender<LOANS>,
    offer: FederatedResourceOffer,
    borrower: NodeId,
    now_us: u64,
    duration_us: u64,
) -> Result<FederatedLease, FederationError> {
    offer.verify(peering_key, now_us)?;
    let peer = directory.peer(offer.provider, now_us)?;
    peer.verify(peering_key, now_us)?;
    let lease_expires_at_us = now_us
        .checked_add(duration_us)
        .ok_or(FederationError::Invalid)?;
    if peer.epoch != offer.epoch
        || peer.gateway != lender.provider()
        || !peer.transports.contains(offer.transports)
        || duration_us == 0
        || lease_expires_at_us > offer.expires_at_us
    {
        return Err(FederationError::AccessDenied);
    }
    let capability = match offer.kind {
        FederatedResourceKind::Cpu => {
            let units = u32::try_from(offer.amount).map_err(|_| FederationError::Invalid)?;
            lender.lend_compute(
                borrower,
                offer.resource,
                units,
                offer.transports,
                now_us,
                duration_us,
            )
        }
        FederatedResourceKind::Ram | FederatedResourceKind::Vram => {
            let range = AddressRange::new(offer.address, offer.amount)
                .map_err(|_| FederationError::Invalid)?;
            lender.lend_memory(
                borrower,
                offer.resource,
                if offer.kind == FederatedResourceKind::Ram {
                    LendingKind::Ram
                } else {
                    LendingKind::Vram
                },
                range,
                LendingRights::READ_WRITE,
                offer.transports,
                now_us,
                duration_us,
            )
        }
    }
    .map_err(FederationError::Lending)?;
    Ok(FederatedLease {
        provider: offer.provider,
        federation_epoch: offer.epoch,
        capability,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RevocationReason {
    LocalPriority = 1,
    LeaseExpired = 2,
    ClusterFailover = 3,
    Policy = 4,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct RevocationSignal {
    pub provider: ClusterId,
    pub resource: u64,
    pub prior_epoch: u64,
    pub next_epoch: u64,
    pub sent_at_us: u64,
    pub deadline_us: u64,
    pub reason: RevocationReason,
    authenticator: [u8; 32],
}

impl fmt::Debug for RevocationSignal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RevocationSignal")
            .field("value", &"[redacted]")
            .finish()
    }
}

impl RevocationSignal {
    pub const WIRE_BYTES: usize = 96;

    #[allow(clippy::too_many_arguments)]
    pub fn issue(
        key: CapabilityKey,
        provider: ClusterId,
        resource: u64,
        prior_epoch: u64,
        next_epoch: u64,
        sent_at_us: u64,
        deadline_us: u64,
        reason: RevocationReason,
    ) -> Result<Self, FederationError> {
        if resource == 0
            || prior_epoch == 0
            || prior_epoch == u64::MAX
            || next_epoch != prior_epoch + 1
            || deadline_us < sent_at_us
            || deadline_us - sent_at_us > MAX_REVOCATION_LATENCY_US
        {
            return Err(FederationError::Invalid);
        }
        let mut signal = Self {
            provider,
            resource,
            prior_epoch,
            next_epoch,
            sent_at_us,
            deadline_us,
            reason,
            authenticator: [0; 32],
        };
        signal.authenticator = key
            .authenticate(&signal.payload())
            .map_err(FederationError::Token)?;
        Ok(signal)
    }

    pub fn apply<const LOANS: usize>(
        &self,
        key: CapabilityKey,
        active_lease: &FederatedLease,
        now_us: u64,
        lender: &mut ResourceLender<LOANS>,
    ) -> Result<RevocationAction, FederationError> {
        key.verify_authenticator(&self.payload(), &self.authenticator)
            .map_err(FederationError::Token)?;
        if self.resource == 0
            || self.prior_epoch == 0
            || self.prior_epoch == u64::MAX
            || self.next_epoch != self.prior_epoch + 1
            || self.deadline_us < self.sent_at_us
            || self.deadline_us - self.sent_at_us > MAX_REVOCATION_LATENCY_US
        {
            return Err(FederationError::Invalid);
        }
        if now_us < self.sent_at_us
            || now_us > self.deadline_us
            || active_lease.provider != self.provider
            || active_lease.capability.resource != self.resource
            || active_lease.capability.revocation_epoch != self.prior_epoch
        {
            return Err(FederationError::StaleEpoch);
        }
        lender
            .revoke(self.resource)
            .map_err(FederationError::Lending)
    }

    pub fn encode(self) -> [u8; Self::WIRE_BYTES] {
        let mut wire = [0; Self::WIRE_BYTES];
        wire[..64].copy_from_slice(&self.payload());
        wire[64..96].copy_from_slice(&self.authenticator);
        wire
    }

    pub fn decode(wire: [u8; Self::WIRE_BYTES]) -> Result<Self, FederationError> {
        if &wire[0..4] != b"SYFR" || wire[4] != 1 {
            return Err(FederationError::Invalid);
        }
        let reason = match wire[5] {
            1 => RevocationReason::LocalPriority,
            2 => RevocationReason::LeaseExpired,
            3 => RevocationReason::ClusterFailover,
            4 => RevocationReason::Policy,
            _ => return Err(FederationError::Invalid),
        };
        let mut authenticator = [0; 32];
        authenticator.copy_from_slice(&wire[64..96]);
        Ok(Self {
            provider: ClusterId::new(read_u128(&wire, 8)).ok_or(FederationError::Invalid)?,
            resource: read_u64(&wire, 24),
            prior_epoch: read_u64(&wire, 32),
            next_epoch: read_u64(&wire, 40),
            sent_at_us: read_u64(&wire, 48),
            deadline_us: read_u64(&wire, 56),
            reason,
            authenticator,
        })
    }

    fn payload(&self) -> [u8; 64] {
        let mut payload = [0; 64];
        payload[0..4].copy_from_slice(b"SYFR");
        payload[4] = 1;
        payload[5] = self.reason as u8;
        payload[8..24].copy_from_slice(&self.provider.raw().to_be_bytes());
        payload[24..32].copy_from_slice(&self.resource.to_be_bytes());
        payload[32..40].copy_from_slice(&self.prior_epoch.to_be_bytes());
        payload[40..48].copy_from_slice(&self.next_epoch.to_be_bytes());
        payload[48..56].copy_from_slice(&self.sent_at_us.to_be_bytes());
        payload[56..64].copy_from_slice(&self.deadline_us.to_be_bytes());
        payload
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FederationError {
    AccessDenied,
    Busy,
    Capacity,
    Duplicate,
    Expired,
    Invalid,
    InvalidState,
    Lending(LendingError),
    Replay,
    StaleEpoch,
    Token(TokenError),
    UnknownCluster,
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

fn read_u128(input: &[u8], offset: usize) -> u128 {
    u128::from_be_bytes([
        input[offset],
        input[offset + 1],
        input[offset + 2],
        input[offset + 3],
        input[offset + 4],
        input[offset + 5],
        input[offset + 6],
        input[offset + 7],
        input[offset + 8],
        input[offset + 9],
        input[offset + 10],
        input[offset + 11],
        input[offset + 12],
        input[offset + 13],
        input[offset + 14],
        input[offset + 15],
    ])
}
