use synos_status::{IntoStatus, Status};

pub const FAULT_DOMAIN_COUNT: usize = 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultDomain {
    Scheduler,
    MemoryManager,
    IpcBroker,
    StorageDaemon,
    NetworkDaemon,
    PackageSupervisor,
}

impl FaultDomain {
    pub const ALL: [Self; FAULT_DOMAIN_COUNT] = [
        Self::Scheduler,
        Self::MemoryManager,
        Self::IpcBroker,
        Self::StorageDaemon,
        Self::NetworkDaemon,
        Self::PackageSupervisor,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Scheduler => "scheduler",
            Self::MemoryManager => "memory-manager",
            Self::IpcBroker => "ipc-broker",
            Self::StorageDaemon => "storage-daemon",
            Self::NetworkDaemon => "network-daemon",
            Self::PackageSupervisor => "package-supervisor",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultCause {
    Panic,
    ProtectionFault,
    Watchdog,
    Corruption,
    ResourceExhaustion,
    ProtocolViolation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultState {
    Healthy,
    Faulted,
    Recovering,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityFence {
    domain: FaultDomain,
    epoch: u64,
    generation: u64,
}

impl CapabilityFence {
    pub const fn domain(self) -> FaultDomain {
        self.domain
    }

    pub const fn epoch(self) -> u64 {
        self.epoch
    }

    pub const fn generation(self) -> u64 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryLease {
    domain: FaultDomain,
    epoch: u64,
}

impl RecoveryLease {
    pub const fn domain(self) -> FaultDomain {
        self.domain
    }

    pub const fn epoch(self) -> u64 {
        self.epoch
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FaultDomainStatus {
    pub domain: FaultDomain,
    pub state: FaultState,
    pub epoch: u64,
    pub generation: u64,
    pub fault_count: u64,
    pub attached_services: usize,
    pub last_cause: Option<FaultCause>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FaultReport {
    pub domain: FaultDomain,
    pub cause: FaultCause,
    pub previous_epoch: u64,
    pub epoch: u64,
    pub generation: u64,
    pub attached_services: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultDomainError {
    InvalidService,
    ServiceAlreadyAttached,
    ServiceCapacity,
    StaleCapability,
    StaleRecoveryLease,
    GenerationExhausted,
    DomainNotFaulted,
    DomainNotRecovering,
}

impl IntoStatus for FaultDomainError {
    fn status(self) -> Status {
        match self {
            Self::InvalidService => Status::INVALID_ARGUMENT,
            Self::ServiceAlreadyAttached | Self::StaleCapability | Self::StaleRecoveryLease => {
                Status::ACCESS_DENIED
            }
            Self::ServiceCapacity | Self::GenerationExhausted => Status::NO_SPACE,
            Self::DomainNotFaulted | Self::DomainNotRecovering => Status::BUSY,
        }
    }
}

#[derive(Clone, Copy)]
struct DomainSlot<const MAX_SERVICES: usize> {
    domain: FaultDomain,
    state: FaultState,
    epoch: u64,
    generation: u64,
    fault_count: u64,
    services: [u64; MAX_SERVICES],
    service_count: usize,
    last_cause: Option<FaultCause>,
}

impl<const MAX_SERVICES: usize> DomainSlot<MAX_SERVICES> {
    const fn new(domain: FaultDomain) -> Self {
        Self {
            domain,
            state: FaultState::Healthy,
            epoch: 1,
            generation: 1,
            fault_count: 0,
            services: [0; MAX_SERVICES],
            service_count: 0,
            last_cause: None,
        }
    }

    const fn status(self) -> FaultDomainStatus {
        FaultDomainStatus {
            domain: self.domain,
            state: self.state,
            epoch: self.epoch,
            generation: self.generation,
            fault_count: self.fault_count,
            attached_services: self.service_count,
            last_cause: self.last_cause,
        }
    }
}

/// Bounded fault fencing for the scheduler, memory manager, IPC broker, and
/// Ring 3 system daemons. Domain state is independent, and every fault bumps
/// both the capability epoch and service generation before recovery starts.
pub struct FaultDomainRegistry<const MAX_SERVICES: usize = 32> {
    domains: [DomainSlot<MAX_SERVICES>; FAULT_DOMAIN_COUNT],
}

impl<const MAX_SERVICES: usize> FaultDomainRegistry<MAX_SERVICES> {
    pub const fn new() -> Self {
        assert!(MAX_SERVICES > 0);
        Self {
            domains: [
                DomainSlot::new(FaultDomain::Scheduler),
                DomainSlot::new(FaultDomain::MemoryManager),
                DomainSlot::new(FaultDomain::IpcBroker),
                DomainSlot::new(FaultDomain::StorageDaemon),
                DomainSlot::new(FaultDomain::NetworkDaemon),
                DomainSlot::new(FaultDomain::PackageSupervisor),
            ],
        }
    }

    pub fn attach(&mut self, domain: FaultDomain, service: u64) -> Result<(), FaultDomainError> {
        if service == 0 {
            return Err(FaultDomainError::InvalidService);
        }
        let slot = self.slot_mut(domain);
        if slot.services[..slot.service_count].contains(&service) {
            return Err(FaultDomainError::ServiceAlreadyAttached);
        }
        let target = slot
            .services
            .get_mut(slot.service_count)
            .ok_or(FaultDomainError::ServiceCapacity)?;
        *target = service;
        slot.service_count += 1;
        Ok(())
    }

    pub fn issue_capability(
        &self,
        domain: FaultDomain,
    ) -> Result<CapabilityFence, FaultDomainError> {
        let slot = self.slot(domain);
        if slot.state != FaultState::Healthy {
            return Err(FaultDomainError::StaleCapability);
        }
        Ok(CapabilityFence {
            domain,
            epoch: slot.epoch,
            generation: slot.generation,
        })
    }

    pub fn validate(&self, fence: CapabilityFence) -> Result<(), FaultDomainError> {
        let slot = self.slot(fence.domain);
        if slot.state != FaultState::Healthy
            || slot.epoch != fence.epoch
            || slot.generation != fence.generation
        {
            return Err(FaultDomainError::StaleCapability);
        }
        Ok(())
    }

    pub fn report_fault(
        &mut self,
        domain: FaultDomain,
        cause: FaultCause,
    ) -> Result<FaultReport, FaultDomainError> {
        let slot = self.slot_mut(domain);
        let previous_epoch = slot.epoch;
        let epoch = slot
            .epoch
            .checked_add(1)
            .ok_or(FaultDomainError::GenerationExhausted)?;
        let generation = slot
            .generation
            .checked_add(1)
            .ok_or(FaultDomainError::GenerationExhausted)?;
        slot.epoch = epoch;
        slot.generation = generation;
        slot.fault_count = slot.fault_count.saturating_add(1);
        slot.state = FaultState::Faulted;
        slot.last_cause = Some(cause);
        Ok(FaultReport {
            domain,
            cause,
            previous_epoch,
            epoch,
            generation,
            attached_services: slot.service_count,
        })
    }

    pub fn begin_recovery(&mut self, domain: FaultDomain) -> Result<RecoveryLease, FaultDomainError> {
        let slot = self.slot_mut(domain);
        if slot.state != FaultState::Faulted {
            return Err(FaultDomainError::DomainNotFaulted);
        }
        slot.state = FaultState::Recovering;
        Ok(RecoveryLease {
            domain,
            epoch: slot.epoch,
        })
    }

    pub fn complete_recovery(
        &mut self,
        lease: RecoveryLease,
    ) -> Result<CapabilityFence, FaultDomainError> {
        let slot = self.slot_mut(lease.domain);
        if slot.state != FaultState::Recovering || slot.epoch != lease.epoch {
            return Err(FaultDomainError::StaleRecoveryLease);
        }
        let epoch = slot
            .epoch
            .checked_add(1)
            .ok_or(FaultDomainError::GenerationExhausted)?;
        let generation = slot
            .generation
            .checked_add(1)
            .ok_or(FaultDomainError::GenerationExhausted)?;
        slot.epoch = epoch;
        slot.generation = generation;
        slot.state = FaultState::Healthy;
        Ok(CapabilityFence {
            domain: lease.domain,
            epoch,
            generation,
        })
    }

    pub fn status(&self, domain: FaultDomain) -> FaultDomainStatus {
        self.slot(domain).status()
    }

    pub fn statuses(&self) -> impl Iterator<Item = FaultDomainStatus> + '_ {
        self.domains.iter().copied().map(DomainSlot::status)
    }

    pub fn service_domain(&self, service: u64) -> Option<FaultDomain> {
        if service == 0 {
            return None;
        }
        self.domains.iter().find_map(|slot| {
            slot.services[..slot.service_count]
                .contains(&service)
                .then_some(slot.domain)
        })
    }

    pub fn accepts_work(&self, domain: FaultDomain) -> bool {
        self.slot(domain).state == FaultState::Healthy
    }

    fn slot(&self, domain: FaultDomain) -> &DomainSlot<MAX_SERVICES> {
        &self.domains[domain.index()]
    }

    fn slot_mut(&mut self, domain: FaultDomain) -> &mut DomainSlot<MAX_SERVICES> {
        &mut self.domains[domain.index()]
    }
}

impl FaultDomain {
    const fn index(self) -> usize {
        match self {
            Self::Scheduler => 0,
            Self::MemoryManager => 1,
            Self::IpcBroker => 2,
            Self::StorageDaemon => 3,
            Self::NetworkDaemon => 4,
            Self::PackageSupervisor => 5,
        }
    }
}

impl<const MAX_SERVICES: usize> Default for FaultDomainRegistry<MAX_SERVICES> {
    fn default() -> Self {
        Self::new()
    }
}
