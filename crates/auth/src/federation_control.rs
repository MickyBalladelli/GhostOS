use core::fmt;

use super::{ClusterId, FederatedResourceKind, FederationError};
use crate::token::{CapabilityKey, TransportRights};

pub const MAX_FEDERATION_RECORDS: usize = 32;
pub const MAX_FEDERATION_INVITATIONS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct FederationScope(u8);

impl FederationScope {
    pub const DISCOVERY: Self = Self(1 << 0);
    pub const CPU: Self = Self(1 << 1);
    pub const RAM: Self = Self(1 << 2);
    pub const VRAM: Self = Self(1 << 3);
    pub const ALL: Self = Self(
        Self::DISCOVERY.0 | Self::CPU.0 | Self::RAM.0 | Self::VRAM.0,
    );

    pub const fn new(bits: u8) -> Option<Self> {
        if bits == 0 || bits & !Self::ALL.0 != 0 {
            None
        } else {
            Some(Self(bits))
        }
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn permits(self, kind: FederatedResourceKind) -> bool {
        match kind {
            FederatedResourceKind::Cpu => self.contains(Self::CPU),
            FederatedResourceKind::Ram => self.contains(Self::RAM),
            FederatedResourceKind::Vram => self.contains(Self::VRAM),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FederationState {
    Invited = 1,
    Pending = 2,
    Active = 3,
    Rejected = 4,
    Expired = 5,
    Revoked = 6,
    Removing = 7,
    Removed = 8,
    Degraded = 9,
    Partitioned = 10,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FederationHealth {
    Unknown = 1,
    Healthy = 2,
    Degraded = 3,
    Partitioned = 4,
    Unavailable = 5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum LeaseOwner {
    None = 0,
    Local = 1,
    Remote = 2,
}

/// Signed cluster-to-cluster admission authority.
///
/// The token contains only cluster identities and an allowed resource scope.
/// It never contains a node identity, filesystem path, process identifier, or
/// socket. Resource capabilities are issued later and remain separately
/// bounded by the accepted scope and federation epoch.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct FederationInvitation {
    pub issuer: ClusterId,
    pub subject: ClusterId,
    pub scope: FederationScope,
    pub transports: TransportRights,
    pub issued_at_us: u64,
    pub expires_at_us: u64,
    pub revocation_epoch: u64,
    pub nonce: u64,
    authenticator: [u8; 32],
}

impl fmt::Debug for FederationInvitation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FederationInvitation")
            .field("value", &"[redacted]")
            .finish()
    }
}

impl FederationInvitation {
    pub const WIRE_BYTES: usize = 112;

    #[allow(clippy::too_many_arguments)]
    pub fn issue(
        key: CapabilityKey,
        issuer: ClusterId,
        subject: ClusterId,
        scope: FederationScope,
        transports: TransportRights,
        issued_at_us: u64,
        expires_at_us: u64,
        revocation_epoch: u64,
        nonce: u64,
    ) -> Result<Self, FederationError> {
        if issuer == subject
            || FederationScope::new(scope.bits()).is_none()
            || expires_at_us <= issued_at_us
            || revocation_epoch == 0
            || nonce == 0
            || TransportRights::from_bits(transports.bits()).is_none()
        {
            return Err(FederationError::Invalid);
        }
        let mut invitation = Self {
            issuer,
            subject,
            scope,
            transports,
            issued_at_us,
            expires_at_us,
            revocation_epoch,
            nonce,
            authenticator: [0; 32],
        };
        invitation.authenticator = key
            .authenticate(&invitation.payload())
            .map_err(FederationError::Token)?;
        Ok(invitation)
    }

    pub fn verify(self, key: CapabilityKey, now_us: u64) -> Result<(), FederationError> {
        if self.issuer == self.subject
            || FederationScope::new(self.scope.bits()).is_none()
            || self.expires_at_us <= self.issued_at_us
            || self.revocation_epoch == 0
            || self.nonce == 0
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
        wire[..72].copy_from_slice(&self.payload());
        wire[72..104].copy_from_slice(&self.authenticator);
        wire
    }

    pub fn decode(wire: [u8; Self::WIRE_BYTES]) -> Result<Self, FederationError> {
        if &wire[0..4] != b"SYFI" || wire[4] != 1 {
            return Err(FederationError::Invalid);
        }
        let scope = FederationScope::new(wire[5]).ok_or(FederationError::Invalid)?;
        let transports = TransportRights::from_bits(wire[6]).ok_or(FederationError::Invalid)?;
        let mut authenticator = [0; 32];
        authenticator.copy_from_slice(&wire[72..104]);
        Ok(Self {
            issuer: ClusterId::new(read_u128(&wire, 8)).ok_or(FederationError::Invalid)?,
            subject: ClusterId::new(read_u128(&wire, 24)).ok_or(FederationError::Invalid)?,
            scope,
            transports,
            issued_at_us: read_u64(&wire, 40),
            expires_at_us: read_u64(&wire, 48),
            revocation_epoch: read_u64(&wire, 56),
            nonce: read_u64(&wire, 64),
            authenticator,
        })
    }

    fn payload(self) -> [u8; 72] {
        let mut payload = [0; 72];
        payload[0..4].copy_from_slice(b"SYFI");
        payload[4] = 1;
        payload[5] = self.scope.bits();
        payload[6] = self.transports.bits();
        payload[8..24].copy_from_slice(&self.issuer.raw().to_be_bytes());
        payload[24..40].copy_from_slice(&self.subject.raw().to_be_bytes());
        payload[40..48].copy_from_slice(&self.issued_at_us.to_be_bytes());
        payload[48..56].copy_from_slice(&self.expires_at_us.to_be_bytes());
        payload[56..64].copy_from_slice(&self.revocation_epoch.to_be_bytes());
        payload[64..72].copy_from_slice(&self.nonce.to_be_bytes());
        payload
    }
}

pub type FederationCapability = FederationInvitation;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FederationRecord {
    pub cluster: ClusterId,
    pub state: FederationState,
    pub scope: FederationScope,
    pub transports: TransportRights,
    pub lease_owner: LeaseOwner,
    pub active_leases: u16,
    pub revocation_epoch: u64,
    pub expires_at_us: u64,
    pub last_seen_us: u64,
    pub health: FederationHealth,
}

impl FederationRecord {
    pub const fn is_active(self) -> bool {
        matches!(self.state, FederationState::Active | FederationState::Degraded)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FederationLeaseStatus {
    pub cluster: ClusterId,
    pub owner: LeaseOwner,
    pub epoch: u64,
    pub active_leases: u16,
}

pub struct FederationRegistry<
    const PEERS: usize = MAX_FEDERATION_RECORDS,
    const INVITATIONS: usize = MAX_FEDERATION_INVITATIONS,
> {
    local: ClusterId,
    key: CapabilityKey,
    records: [Option<FederationRecord>; PEERS],
    invitations: [Option<FederationInvitation>; INVITATIONS],
}

impl<const PEERS: usize, const INVITATIONS: usize> FederationRegistry<PEERS, INVITATIONS> {
    pub const fn new(local: ClusterId, key: CapabilityKey) -> Self {
        Self {
            local,
            key,
            records: [None; PEERS],
            invitations: [None; INVITATIONS],
        }
    }

    pub const fn local(&self) -> ClusterId {
        self.local
    }

    pub fn records(&self) -> impl Iterator<Item = FederationRecord> + '_ {
        self.records.iter().flatten().copied()
    }

    pub fn record(&self, cluster: ClusterId) -> Option<FederationRecord> {
        self.records
            .iter()
            .flatten()
            .find(|record| record.cluster == cluster)
            .copied()
    }

    pub fn invitation(&self, nonce: u64) -> Option<FederationInvitation> {
        self.invitations
            .iter()
            .flatten()
            .find(|invitation| invitation.nonce == nonce)
            .copied()
    }

    pub fn invite(
        &mut self,
        peer: ClusterId,
        scope: FederationScope,
        transports: TransportRights,
        now_us: u64,
        duration_us: u64,
        nonce: u64,
    ) -> Result<FederationInvitation, FederationError> {
        if peer == self.local || duration_us == 0 || self.record(peer).is_some() {
            return Err(if peer == self.local || duration_us == 0 {
                FederationError::Invalid
            } else {
                FederationError::Duplicate
            });
        }
        if self
            .invitations
            .iter()
            .flatten()
            .any(|invitation| invitation.nonce == nonce)
        {
            return Err(FederationError::Replay);
        }
        if !self.records.iter().any(|record| record.is_none()) {
            return Err(FederationError::Capacity);
        }
        let expires_at_us = now_us
            .checked_add(duration_us)
            .ok_or(FederationError::Invalid)?;
        let invitation = FederationInvitation::issue(
            self.key,
            self.local,
            peer,
            scope,
            transports,
            now_us,
            expires_at_us,
            1,
            nonce,
        )?;
        self.store_record(FederationRecord {
            cluster: peer,
            state: FederationState::Invited,
            scope,
            transports,
            lease_owner: LeaseOwner::None,
            active_leases: 0,
            revocation_epoch: 1,
            expires_at_us,
            last_seen_us: now_us,
            health: FederationHealth::Unknown,
        })?;
        self.store_invitation(invitation)?;
        Ok(invitation)
    }

    pub fn invite_cluster(
        &mut self,
        peer: ClusterId,
        scope: FederationScope,
        transports: TransportRights,
        now_us: u64,
        duration_us: u64,
        nonce: u64,
    ) -> Result<FederationInvitation, FederationError> {
        self.invite(peer, scope, transports, now_us, duration_us, nonce)
    }

    /// Accept an invitation on either side of the relationship. The issuer
    /// records the returned acceptance as active; the subject creates its
    /// active peer record. Both paths require the signed invitation.
    pub fn accept(
        &mut self,
        invitation: FederationInvitation,
        now_us: u64,
    ) -> Result<FederationRecord, FederationError> {
        invitation.verify(self.key, now_us)?;
        let peer = if invitation.subject == self.local {
            invitation.issuer
        } else if invitation.issuer == self.local {
            invitation.subject
        } else {
            return Err(FederationError::AccessDenied);
        };
        if self
            .record(peer)
            .is_some_and(|record| record.state == FederationState::Active)
        {
            return Err(FederationError::Replay);
        }
        let record = FederationRecord {
            cluster: peer,
            state: FederationState::Active,
            scope: invitation.scope,
            transports: invitation.transports,
            lease_owner: LeaseOwner::None,
            active_leases: 0,
            revocation_epoch: invitation.revocation_epoch,
            expires_at_us: invitation.expires_at_us,
            last_seen_us: now_us,
            health: FederationHealth::Healthy,
        };
        self.upsert_record(record)?;
        self.store_invitation(invitation)?;
        Ok(record)
    }

    pub fn accept_cluster(
        &mut self,
        invitation: FederationInvitation,
        now_us: u64,
    ) -> Result<FederationRecord, FederationError> {
        self.accept(invitation, now_us)
    }

    pub fn reject(
        &mut self,
        invitation: FederationInvitation,
        now_us: u64,
    ) -> Result<(), FederationError> {
        invitation.verify(self.key, now_us)?;
        let peer = if invitation.subject == self.local {
            invitation.issuer
        } else if invitation.issuer == self.local {
            invitation.subject
        } else {
            return Err(FederationError::AccessDenied);
        };
        self.upsert_record(FederationRecord {
            cluster: peer,
            state: FederationState::Rejected,
            scope: invitation.scope,
            transports: invitation.transports,
            lease_owner: LeaseOwner::None,
            active_leases: 0,
            revocation_epoch: invitation.revocation_epoch,
            expires_at_us: invitation.expires_at_us,
            last_seen_us: now_us,
            health: FederationHealth::Unavailable,
        })?;
        self.store_invitation(invitation)
    }

    pub fn reject_cluster(
        &mut self,
        invitation: FederationInvitation,
        now_us: u64,
    ) -> Result<(), FederationError> {
        self.reject(invitation, now_us)
    }

    pub fn remove(
        &mut self,
        peer: ClusterId,
        force: bool,
        now_us: u64,
    ) -> Result<FederationRecord, FederationError> {
        let slot = self.slot(peer).ok_or(FederationError::UnknownCluster)?;
        let record = self.records[slot].ok_or(FederationError::UnknownCluster)?;
        if record.active_leases != 0 && !force {
            return Err(FederationError::Busy);
        }
        let mut record = record;
        record.state = FederationState::Removed;
        record.lease_owner = LeaseOwner::None;
        record.active_leases = 0;
        record.revocation_epoch = next_epoch(record.revocation_epoch)?;
        record.last_seen_us = now_us;
        record.health = FederationHealth::Unavailable;
        self.records[slot] = Some(record);
        Ok(record)
    }

    pub fn remove_federation(
        &mut self,
        peer: ClusterId,
        force: bool,
        now_us: u64,
    ) -> Result<FederationRecord, FederationError> {
        self.remove(peer, force, now_us)
    }

    /// Expire invitations and relationships without granting any new access.
    /// Callers can run this from their normal timer tick.
    pub fn refresh(&mut self, now_us: u64) -> usize {
        let mut expired = 0;
        for record in &mut self.records {
            let Some(record) = record.as_mut() else {
                continue
            };
            if record.expires_at_us <= now_us
                && !matches!(
                    record.state,
                    FederationState::Expired
                        | FederationState::Removed
                        | FederationState::Revoked
                        | FederationState::Rejected
                )
            {
                record.state = FederationState::Expired;
                record.health = FederationHealth::Unavailable;
                record.active_leases = 0;
                record.lease_owner = LeaseOwner::None;
                expired += 1;
            }
        }
        expired
    }

    pub fn set_health(
        &mut self,
        peer: ClusterId,
        health: FederationHealth,
        now_us: u64,
    ) -> Result<FederationRecord, FederationError> {
        let slot = self.slot(peer).ok_or(FederationError::UnknownCluster)?;
        let mut record = self.records[slot].ok_or(FederationError::UnknownCluster)?;
        if !record.is_active() {
            return Err(FederationError::InvalidState);
        }
        record.health = health;
        record.state = match health {
            FederationHealth::Healthy | FederationHealth::Unknown => FederationState::Active,
            FederationHealth::Degraded => FederationState::Degraded,
            FederationHealth::Partitioned => FederationState::Partitioned,
            FederationHealth::Unavailable => FederationState::Revoked,
        };
        record.last_seen_us = now_us;
        self.records[slot] = Some(record);
        Ok(record)
    }

    pub fn authorize_offer(
        &self,
        peer: ClusterId,
        kind: FederatedResourceKind,
        transports: TransportRights,
        epoch: u64,
        now_us: u64,
    ) -> Result<(), FederationError> {
        let record = self.record(peer).ok_or(FederationError::UnknownCluster)?;
        if !record.is_active()
            || record.health == FederationHealth::Partitioned
            || record.health == FederationHealth::Unavailable
            || now_us >= record.expires_at_us
            || epoch != record.revocation_epoch
            || !record.scope.permits(kind)
            || !record.transports.contains(transports)
        {
            return Err(if now_us >= record.expires_at_us {
                FederationError::Expired
            } else if epoch != record.revocation_epoch {
                FederationError::StaleEpoch
            } else {
                FederationError::AccessDenied
            });
        }
        Ok(())
    }

    pub fn acquire_lease(
        &mut self,
        peer: ClusterId,
        owner: LeaseOwner,
        kind: FederatedResourceKind,
        transports: TransportRights,
        epoch: u64,
        now_us: u64,
    ) -> Result<FederationLeaseStatus, FederationError> {
        self.authorize_offer(peer, kind, transports, epoch, now_us)?;
        let slot = self.slot(peer).ok_or(FederationError::UnknownCluster)?;
        let mut record = self.records[slot].ok_or(FederationError::UnknownCluster)?;
        record.active_leases = record
            .active_leases
            .checked_add(1)
            .ok_or(FederationError::Capacity)?;
        record.lease_owner = owner;
        record.last_seen_us = now_us;
        self.records[slot] = Some(record);
        Ok(FederationLeaseStatus {
            cluster: peer,
            owner,
            epoch: record.revocation_epoch,
            active_leases: record.active_leases,
        })
    }

    pub fn release_lease(&mut self, peer: ClusterId) -> Result<(), FederationError> {
        let slot = self.slot(peer).ok_or(FederationError::UnknownCluster)?;
        let mut record = self.records[slot].ok_or(FederationError::UnknownCluster)?;
        if record.active_leases == 0 {
            return Err(FederationError::InvalidState);
        }
        record.active_leases -= 1;
        if record.active_leases == 0 {
            record.lease_owner = LeaseOwner::None;
        }
        self.records[slot] = Some(record);
        Ok(())
    }

    /// Advance the peer epoch before sending a revocation. All old offers,
    /// capabilities, and DLM leases then fail the epoch check.
    pub fn fence(&mut self, peer: ClusterId, now_us: u64) -> Result<u64, FederationError> {
        let slot = self.slot(peer).ok_or(FederationError::UnknownCluster)?;
        let mut record = self.records[slot].ok_or(FederationError::UnknownCluster)?;
        record.revocation_epoch = next_epoch(record.revocation_epoch)?;
        record.active_leases = 0;
        record.lease_owner = LeaseOwner::None;
        record.last_seen_us = now_us;
        self.records[slot] = Some(record);
        Ok(record.revocation_epoch)
    }

    pub fn preempt(&mut self, peer: ClusterId, now_us: u64) -> Result<u64, FederationError> {
        self.fence(peer, now_us)
    }

    pub fn validate_epoch(
        &self,
        peer: ClusterId,
        epoch: u64,
        now_us: u64,
    ) -> Result<(), FederationError> {
        let record = self.record(peer).ok_or(FederationError::UnknownCluster)?;
        if now_us >= record.expires_at_us {
            Err(FederationError::Expired)
        } else if !record.is_active() {
            Err(FederationError::AccessDenied)
        } else if record.revocation_epoch != epoch {
            Err(FederationError::StaleEpoch)
        } else {
            Ok(())
        }
    }

    fn slot(&self, peer: ClusterId) -> Option<usize> {
        self.records
            .iter()
            .position(|record| record.is_some_and(|record| record.cluster == peer))
    }

    fn store_record(&mut self, record: FederationRecord) -> Result<(), FederationError> {
        let slot = self
            .records
            .iter_mut()
            .position(|record| record.is_none())
            .ok_or(FederationError::Capacity)?;
        self.records[slot] = Some(record);
        Ok(())
    }

    fn upsert_record(&mut self, record: FederationRecord) -> Result<(), FederationError> {
        if let Some(slot) = self.slot(record.cluster) {
            self.records[slot] = Some(record);
            Ok(())
        } else {
            self.store_record(record)
        }
    }

    fn store_invitation(&mut self, invitation: FederationInvitation) -> Result<(), FederationError> {
        if self
            .invitations
            .iter()
            .flatten()
            .any(|existing| *existing == invitation)
        {
            return Ok(());
        }
        if self
            .invitations
            .iter()
            .flatten()
            .any(|existing| existing.nonce == invitation.nonce)
        {
            return Err(FederationError::Replay);
        }
        let slot = self
            .invitations
            .iter_mut()
            .position(|invitation| invitation.is_none())
            .ok_or(FederationError::Capacity)?;
        self.invitations[slot] = Some(invitation);
        Ok(())
    }
}

impl<const PEERS: usize, const INVITATIONS: usize> Default
    for FederationRegistry<PEERS, INVITATIONS>
{
    fn default() -> Self {
        Self::new(
            ClusterId::new(1).expect("non-zero default cluster"),
            CapabilityKey::new([0; 32]),
        )
    }
}

fn next_epoch(epoch: u64) -> Result<u64, FederationError> {
    epoch.checked_add(1).filter(|next| *next != 0).ok_or(FederationError::Invalid)
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
