use synos_fabric::NodeId;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct PrincipalId(u64);

impl PrincipalId {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct InspectionRights(u8);

impl InspectionRights {
    pub const NONE: Self = Self(0);
    pub const MEMORY: Self = Self(1 << 0);
    pub const STORAGE: Self = Self(1 << 1);
    pub const CPU: Self = Self(1 << 2);
    pub const ACTIVITY: Self = Self(1 << 3);
    pub const AUDIT_WORLD: Self = Self(1 << 4);
    pub const OBSOLESCENCE: Self = Self(1 << 5);
    pub const HEALTH: Self = Self(1 << 6);
    pub const LOCAL_DIAGNOSTICS: Self = Self(
        Self::MEMORY.0
            | Self::STORAGE.0
            | Self::CPU.0
            | Self::ACTIVITY.0
            | Self::OBSOLESCENCE.0
            | Self::HEALTH.0,
    );
    pub const ALL: Self = Self(Self::LOCAL_DIAGNOSTICS.0 | Self::AUDIT_WORLD.0);

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

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

pub const CAP_AUDIT_WORLD: InspectionRights = InspectionRights::AUDIT_WORLD;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InspectCapability {
    issuer: u64,
    epoch: u64,
    subject: PrincipalId,
    home_node: NodeId,
    rights: InspectionRights,
    expires_at_us: u64,
}

impl InspectCapability {
    pub const fn subject(self) -> PrincipalId {
        self.subject
    }

    pub const fn home_node(self) -> NodeId {
        self.home_node
    }

    pub const fn rights(self) -> InspectionRights {
        self.rights
    }

    pub const fn expires_at_us(self) -> u64 {
        self.expires_at_us
    }
}

/// Trusted issuer for opaque inspection capabilities.
///
/// The issuer identifier is supplied by the kernel capability manager. A
/// service validates both it and the revocation epoch before exposing data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InspectionAuthority {
    issuer: u64,
    epoch: u64,
}

impl InspectionAuthority {
    pub const fn new(issuer: u64, epoch: u64) -> Option<Self> {
        if issuer == 0 || epoch == 0 {
            None
        } else {
            Some(Self { issuer, epoch })
        }
    }

    pub const fn epoch(self) -> u64 {
        self.epoch
    }

    pub fn revoke_all(&mut self) -> u64 {
        self.epoch = self.epoch.wrapping_add(1).max(1);
        self.epoch
    }

    pub const fn issue(
        self,
        subject: PrincipalId,
        home_node: NodeId,
        rights: InspectionRights,
        expires_at_us: u64,
    ) -> Option<InspectCapability> {
        if rights.bits() == 0 || expires_at_us == 0 {
            return None
        }
        Some(InspectCapability {
            issuer: self.issuer,
            epoch: self.epoch,
            subject,
            home_node,
            rights,
            expires_at_us,
        })
    }

    pub(crate) const fn authorizes(
        self,
        capability: InspectCapability,
        required: InspectionRights,
        now_us: u64,
    ) -> bool {
        capability.issuer == self.issuer
            && capability.epoch == self.epoch
            && now_us < capability.expires_at_us
            && capability.rights.contains(required)
    }
}
