use ghostos_fabric::{AddressRange, PAGE_SIZE};
use ghostos_ipc::InheritableDescriptor;
use ghostos_system_model::ContentId;

use crate::Error;

const TOKEN_BYTES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtectedResource {
    SharedDsm(AddressRange),
    Ipc(InheritableDescriptor),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct CapabilityRights(u8);

impl CapabilityRights {
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const SEND: Self = Self(1 << 2);
    pub const RECEIVE: Self = Self(1 << 3);
    pub const ALL: Self = Self(Self::READ.0 | Self::WRITE.0 | Self::SEND.0 | Self::RECEIVE.0);

    pub const fn from_bits(bits: u8) -> Option<Self> {
        if bits & !Self::ALL.0 == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfidentialCapability {
    issuer: u64,
    epoch: u64,
    id: u64,
    node: ghostos_fabric::NodeId,
    subject: u64,
    resource: ProtectedResource,
    rights: CapabilityRights,
    expires_at_us: u64,
    attestation: [u8; TOKEN_BYTES],
    token: [u8; TOKEN_BYTES],
}

impl ConfidentialCapability {
    pub const fn id(self) -> u64 {
        self.id
    }

    pub const fn node(self) -> ghostos_fabric::NodeId {
        self.node
    }

    pub const fn subject(self) -> u64 {
        self.subject
    }

    pub const fn resource(self) -> ProtectedResource {
        self.resource
    }

    pub const fn rights(self) -> CapabilityRights {
        self.rights
    }

    pub const fn expires_at_us(self) -> u64 {
        self.expires_at_us
    }

    pub const fn attestation(self) -> [u8; TOKEN_BYTES] {
        self.attestation
    }

    pub const fn token(self) -> [u8; TOKEN_BYTES] {
        self.token
    }
}

pub struct CapabilityAuthority<const CAPACITY: usize = 128> {
    issuer: u64,
    epoch: u64,
    next_id: u64,
    issued: [Option<ConfidentialCapability>; CAPACITY],
}

impl<const CAPACITY: usize> CapabilityAuthority<CAPACITY> {
    pub fn new(issuer: u64) -> Option<Self> {
        (issuer != 0 && CAPACITY != 0).then_some(Self {
            issuer,
            epoch: 1,
            next_id: 1,
            issued: [None; CAPACITY],
        })
    }

    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn issue(
        &mut self,
        node: ghostos_fabric::NodeId,
        subject: u64,
        resource: ProtectedResource,
        rights: CapabilityRights,
        expires_at_us: u64,
        now_us: u64,
        attestation: [u8; TOKEN_BYTES],
    ) -> Result<ConfidentialCapability, Error> {
        if subject == 0
            || rights == CapabilityRights::NONE
            || expires_at_us == 0
            || expires_at_us <= now_us
            || attestation.iter().all(|byte| *byte == 0)
            || !valid_resource(resource)
        {
            return Err(Error::InvalidInput)
        }
        let slot = self
            .issued
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(Error::Capacity)?;
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let token = token_digest(
            self.issuer,
            self.epoch,
            id,
            node,
            subject,
            resource,
            rights,
            expires_at_us,
            attestation,
        );
        let capability = ConfidentialCapability {
            issuer: self.issuer,
            epoch: self.epoch,
            id,
            node,
            subject,
            resource,
            rights,
            expires_at_us,
            attestation,
            token,
        };
        *slot = Some(capability);
        Ok(capability)
    }

    pub fn validate(
        &self,
        capability: ConfidentialCapability,
        subject: u64,
        required: CapabilityRights,
        now_us: u64,
    ) -> Result<(), Error> {
        if capability.issuer != self.issuer
            || capability.epoch != self.epoch
            || capability.subject != subject
            || now_us >= capability.expires_at_us
            || !capability.rights.contains(required)
            || !self
                .issued
                .iter()
                .flatten()
                .any(|entry| *entry == capability)
        {
            return Err(if now_us >= capability.expires_at_us {
                Error::Expired
            } else {
                Error::Unauthorized
            })
        }
        let expected = token_digest(
            capability.issuer,
            capability.epoch,
            capability.id,
            capability.node,
            capability.subject,
            capability.resource,
            capability.rights,
            capability.expires_at_us,
            capability.attestation,
        );
        if expected != capability.token {
            return Err(Error::AuthenticationFailed)
        }
        Ok(())
    }

    pub fn revoke(&mut self, capability: ConfidentialCapability) -> Result<(), Error> {
        let slot = self
            .issued
            .iter_mut()
            .find(|entry| entry.is_some_and(|entry| entry == capability))
            .ok_or(Error::NotFound)?;
        *slot = None;
        Ok(())
    }

    pub fn revoke_all(&mut self) -> u64 {
        self.epoch = self.epoch.wrapping_add(1).max(1);
        self.issued.fill(None);
        self.epoch
    }
}

fn valid_resource(resource: ProtectedResource) -> bool {
    match resource {
        ProtectedResource::SharedDsm(range) => {
            range.start % PAGE_SIZE == 0
                && range.length % PAGE_SIZE == 0
                && range.length != 0
                && range.start.checked_add(range.length).is_some()
        }
        ProtectedResource::Ipc(InheritableDescriptor::Channel(id)) => id.raw() != 0,
        ProtectedResource::Ipc(InheritableDescriptor::SharedRegion(id)) => id.raw() != 0,
    }
}

fn token_digest(
    issuer: u64,
    epoch: u64,
    id: u64,
    node: ghostos_fabric::NodeId,
    subject: u64,
    resource: ProtectedResource,
    rights: CapabilityRights,
    expires_at_us: u64,
    attestation: [u8; TOKEN_BYTES],
) -> [u8; TOKEN_BYTES] {
    let mut material = [0; 128];
    material[0..8].copy_from_slice(&issuer.to_be_bytes());
    material[8..16].copy_from_slice(&epoch.to_be_bytes());
    material[16..24].copy_from_slice(&id.to_be_bytes());
    material[24..28].copy_from_slice(&node.raw().to_be_bytes());
    material[28..36].copy_from_slice(&subject.to_be_bytes());
    material[36] = rights.bits();
    material[37..45].copy_from_slice(&expires_at_us.to_be_bytes());
    material[45..77].copy_from_slice(&attestation);
    encode_resource(resource, &mut material[77..]);
    *ContentId::hash(&material).as_bytes()
}

fn encode_resource(resource: ProtectedResource, output: &mut [u8]) {
    match resource {
        ProtectedResource::SharedDsm(range) => {
            output[0] = 1;
            output[1..9].copy_from_slice(&range.start.to_be_bytes());
            output[9..17].copy_from_slice(&range.length.to_be_bytes());
        }
        ProtectedResource::Ipc(InheritableDescriptor::Channel(id)) => {
            output[0] = 2;
            output[1..5].copy_from_slice(&id.raw().to_be_bytes());
        }
        ProtectedResource::Ipc(InheritableDescriptor::SharedRegion(id)) => {
            output[0] = 3;
            output[1..5].copy_from_slice(&id.raw().to_be_bytes());
        }
    }
}
