use crate::service::StoragePath;
use synos_auth::{CapabilityCaveat, CapabilityKey, CryptographicCapability, TokenError, TransportRights};
use synos_fabric::NodeId;
use synos_kernel::Rights;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityError {
    Invalid,
    Expired,
    Revoked,
    WrongMount,
    WrongScope,
    RightsDenied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageRights(u16);

impl StorageRights {
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const MOUNT: Self = Self(1 << 2);
    pub const UNMOUNT: Self = Self(1 << 3);
    pub const ADMIN: Self = Self(1 << 4);
    pub const STREAM: Self = Self(1 << 5);
    pub const ALL: Self = Self((1 << 6) - 1);

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}

/// Opaque daemon-issued authority. Its tag is checked against the daemon's
/// private issuer secret; callers can only reduce rights through attenuation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageCapability {
    token: CryptographicCapability,
    rights: StorageRights,
    scope: StoragePath,
    expires_at: u64,
}

impl StorageCapability {
    pub const fn rights(self) -> StorageRights {
        self.rights
    }

    pub const fn mount(self) -> u32 {
        self.token.resource as u32
    }

    pub const fn expires_at(self) -> u64 {
        self.expires_at
    }

    pub fn scope(self) -> StoragePath {
        self.scope
    }

    pub fn attenuate(
        self,
        rights: StorageRights,
        scope: StoragePath,
        expires_at: u64,
    ) -> Result<Self, CapabilityError> {
        if !self.rights.contains(rights)
            || !self.scope.contains(scope)
            || expires_at == 0
            || expires_at > self.expires_at
        {
            return Err(CapabilityError::RightsDenied)
        }
        let token = self
            .token
            .attenuate(CapabilityCaveat {
                subject: None,
                rights: kernel_rights(rights),
                transports: TransportRights::LAYER2,
                expires_at_us: expires_at,
            })
            .map_err(map_token_error)?;
        Ok(Self {
            token,
            rights,
            scope,
            expires_at,
        })
    }

    pub(crate) fn issue(
        mount: u32,
        generation: u32,
        rights: StorageRights,
        scope: StoragePath,
        expires_at: u64,
        secret: u64,
    ) -> Result<Self, CapabilityError> {
        if mount == 0 || generation == 0 || rights.0 == 0 || expires_at == 0 {
            return Err(CapabilityError::Invalid)
        }
        let token = CryptographicCapability::issue(
            key(secret),
            node(),
            node(),
            mount as u64,
            kernel_rights(rights),
            TransportRights::LAYER2,
            0,
            expires_at,
            generation as u64,
            u64::from(mount) << 32 | u64::from(generation),
        )
        .map_err(map_token_error)?;
        Ok(Self {
            token,
            rights,
            scope,
            expires_at,
        })
    }

    pub(crate) fn verify(
        self,
        mount: u32,
        generation: u32,
        path: StoragePath,
        required: StorageRights,
        now: u64,
        secret: u64,
    ) -> Result<(), CapabilityError> {
        if self.mount() != mount {
            return Err(CapabilityError::WrongMount)
        }
        if !self.rights.contains(required) {
            return Err(CapabilityError::RightsDenied)
        }
        if !self.scope.contains(path) {
            return Err(CapabilityError::WrongScope)
        }
        self.token
            .verify(
                key(secret),
                node(),
                kernel_rights(required),
                TransportRights::LAYER2,
                now,
                generation as u64,
            )
            .map_err(map_token_error)
    }
}

fn key(secret: u64) -> CapabilityKey {
    let bytes = secret.to_le_bytes();
    let mut key = [0; 32];
    key[..8].copy_from_slice(&bytes);
    key[8..16].copy_from_slice(&bytes);
    key[16..24].copy_from_slice(&bytes);
    key[24..32].copy_from_slice(&bytes);
    CapabilityKey::new(key)
}

fn node() -> NodeId {
    NodeId::from_valid_raw(1)
}

fn kernel_rights(rights: StorageRights) -> Rights {
    let mut result = Rights::NONE;
    if rights.contains(StorageRights::READ) {
        result = result.union(Rights::READ)
    }
    if rights.contains(StorageRights::WRITE) {
        result = result.union(Rights::WRITE)
    }
    if rights.contains(StorageRights::MOUNT) || rights.contains(StorageRights::UNMOUNT) {
        result = result.union(Rights::CONTROL)
    }
    if rights.contains(StorageRights::ADMIN) {
        result = result.union(Rights::CONTROL)
    }
    if rights.contains(StorageRights::STREAM) {
        result = result.union(Rights::READ)
    }
    result
}

fn map_token_error(error: TokenError) -> CapabilityError {
    match error {
        TokenError::AccessDenied => CapabilityError::RightsDenied,
        TokenError::InvalidSignature | TokenError::Invalid => CapabilityError::Invalid,
        TokenError::RightsEscalation | TokenError::CaveatCapacity => CapabilityError::RightsDenied,
    }
}
