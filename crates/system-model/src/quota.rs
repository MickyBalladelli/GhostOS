use ghostos_status::{IntoStatus, Status};

pub const QUOTA_RESOURCE_COUNT: usize = 8;

/// Resource dimensions shared by admission, services, and observability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum QuotaResource {
    Memory = 0,
    Cpu = 1,
    Ipc = 2,
    Storage = 3,
    Network = 4,
    Log = 5,
    Audit = 6,
    ControlPlane = 7,
}

impl QuotaResource {
    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Cpu => "cpu",
            Self::Ipc => "ipc",
            Self::Storage => "storage",
            Self::Network => "network",
            Self::Log => "log",
            Self::Audit => "audit",
            Self::ControlPlane => "control-plane",
        }
    }
}

/// A charge applied to one quota dimension.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuotaCharge {
    pub resource: QuotaResource,
    pub amount: u64,
}

impl QuotaCharge {
    pub const fn new(resource: QuotaResource, amount: u64) -> Self {
        Self { resource, amount }
    }
}

/// Fixed policy limits for all resource dimensions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuotaPolicy {
    limits: [u64; QUOTA_RESOURCE_COUNT],
}

impl QuotaPolicy {
    pub const UNLIMITED: Self = Self {
        limits: [u64::MAX; QUOTA_RESOURCE_COUNT],
    };

    pub const fn new(limits: [u64; QUOTA_RESOURCE_COUNT]) -> Self {
        Self { limits }
    }

    pub const fn limit(self, resource: QuotaResource) -> u64 {
        self.limits[resource.index()]
    }

    pub const fn limits(self) -> [u64; QUOTA_RESOURCE_COUNT] {
        self.limits
    }
}

impl Default for QuotaPolicy {
    fn default() -> Self {
        Self::UNLIMITED
    }
}

/// Structured reason why a quota operation was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuotaRejection {
    LimitExceeded {
        resource: QuotaResource,
        requested: u64,
        consumed: u64,
        limit: u64,
    },
    CounterOverflow {
        resource: QuotaResource,
        requested: u64,
        consumed: u64,
    },
    ReleaseExceedsConsumption {
        resource: QuotaResource,
        released: u64,
        consumed: u64,
    },
}

impl QuotaRejection {
    pub const fn resource(self) -> QuotaResource {
        match self {
            Self::LimitExceeded { resource, .. }
            | Self::CounterOverflow { resource, .. }
            | Self::ReleaseExceedsConsumption { resource, .. } => resource,
        }
    }
}

impl IntoStatus for QuotaRejection {
    fn status(self) -> Status {
        match self {
            Self::LimitExceeded { .. } | Self::CounterOverflow { .. } => Status::NO_SPACE,
            Self::ReleaseExceedsConsumption { .. } => Status::INVALID_ARGUMENT,
        }
    }
}

/// Current usage for all quota dimensions.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct QuotaUsage {
    consumed: [u64; QUOTA_RESOURCE_COUNT],
}

impl QuotaUsage {
    pub const fn consumed(self, resource: QuotaResource) -> u64 {
        self.consumed[resource.index()]
    }

    pub const fn consumption(self) -> [u64; QUOTA_RESOURCE_COUNT] {
        self.consumed
    }
}

/// Fixed-size quota accounting with inspectable consumption.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuotaLedger {
    policy: QuotaPolicy,
    usage: QuotaUsage,
}

impl QuotaLedger {
    pub const fn new(policy: QuotaPolicy) -> Self {
        Self {
            policy,
            usage: QuotaUsage {
                consumed: [0; QUOTA_RESOURCE_COUNT],
            },
        }
    }

    pub const fn policy(self) -> QuotaPolicy {
        self.policy
    }

    pub const fn usage(self) -> QuotaUsage {
        self.usage
    }

    pub const fn remaining(self, resource: QuotaResource) -> u64 {
        self.policy
            .limit(resource)
            .saturating_sub(self.usage.consumed(resource))
    }

    /// Reserve one charge. A rejected charge does not change usage.
    pub fn reserve(
        &mut self,
        resource: QuotaResource,
        amount: u64,
    ) -> Result<(), QuotaRejection> {
        let mut next = self.usage;
        reserve_usage(&mut next, self.policy, resource, amount)?;
        self.usage = next;
        Ok(())
    }

    /// Reserve several charges atomically. No usage changes if any charge fails.
    pub fn reserve_all(&mut self, charges: &[QuotaCharge]) -> Result<(), QuotaRejection> {
        let mut next = self.usage;
        for charge in charges {
            reserve_usage(&mut next, self.policy, charge.resource, charge.amount)?;
        }
        self.usage = next;
        Ok(())
    }

    pub fn release(
        &mut self,
        resource: QuotaResource,
        amount: u64,
    ) -> Result<(), QuotaRejection> {
        let consumed = self.usage.consumed(resource);
        if amount > consumed {
            return Err(QuotaRejection::ReleaseExceedsConsumption {
                resource,
                released: amount,
                consumed,
            });
        }
        self.usage.consumed[resource.index()] = consumed - amount;
        Ok(())
    }

    pub fn release_all(&mut self, charges: &[QuotaCharge]) -> Result<(), QuotaRejection> {
        let mut next = self.usage;
        for charge in charges {
            let consumed = next.consumed(charge.resource);
            if charge.amount > consumed {
                return Err(QuotaRejection::ReleaseExceedsConsumption {
                    resource: charge.resource,
                    released: charge.amount,
                    consumed,
                });
            }
            next.consumed[charge.resource.index()] = consumed - charge.amount;
        }
        self.usage = next;
        Ok(())
    }
}

impl Default for QuotaLedger {
    fn default() -> Self {
        Self::new(QuotaPolicy::UNLIMITED)
    }
}

fn reserve_usage(
    usage: &mut QuotaUsage,
    policy: QuotaPolicy,
    resource: QuotaResource,
    amount: u64,
) -> Result<(), QuotaRejection> {
    let consumed = usage.consumed(resource);
    let next = consumed
        .checked_add(amount)
        .ok_or(QuotaRejection::CounterOverflow {
            resource,
            requested: amount,
            consumed,
        })?;
    let limit = policy.limit(resource);
    if next > limit {
        return Err(QuotaRejection::LimitExceeded {
            resource,
            requested: amount,
            consumed,
            limit,
        });
    }
    usage.consumed[resource.index()] = next;
    Ok(())
}
