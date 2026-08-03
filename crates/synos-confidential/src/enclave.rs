use synos_fabric::{AddressRange, NodeId, PAGE_SIZE};
use synos_ipc::InheritableDescriptor;
use synos_observability::{EventField, Level, audit_event, field};
use synos_shield::attestation::{AdmissionController, AttestationKey, AttestationQuote, HardwareRoot};

use crate::capability::{CapabilityAuthority, CapabilityRights, ConfidentialCapability, ProtectedResource};
use crate::Error;

pub const DEFAULT_ENCLAVE_CAPACITY: usize = 32;
pub const DEFAULT_MEMORY_RANGE_CAPACITY: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnclaveClass {
    Cpu,
    Gpu,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnclavePlatform {
    AmdSevSnp,
    IntelTdx,
    NvidiaTee,
}

impl EnclavePlatform {
    pub const fn class(self) -> EnclaveClass {
        match self {
            Self::NvidiaTee => EnclaveClass::Gpu,
            Self::AmdSevSnp | Self::IntelTdx => EnclaveClass::Cpu,
        }
    }

    const fn hardware_root(self) -> HardwareRoot {
        match self {
            Self::AmdSevSnp => HardwareRoot::AmdSevSnp,
            Self::IntelTdx => HardwareRoot::IntelTdx,
            Self::NvidiaTee => HardwareRoot::NvidiaTee,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnclaveBinding<const RANGES: usize = DEFAULT_MEMORY_RANGE_CAPACITY> {
    pub node: NodeId,
    pub platform: EnclavePlatform,
    pub measurement: [u8; 32],
    ranges: [Option<AddressRange>; RANGES],
    range_count: usize,
    admitted: bool,
}

impl<const RANGES: usize> EnclaveBinding<RANGES> {
    pub fn new(
        node: NodeId,
        platform: EnclavePlatform,
        measurement: [u8; 32],
        ranges: &[AddressRange],
    ) -> Result<Self, Error> {
        if measurement.iter().all(|byte| *byte == 0)
            || ranges.is_empty()
            || ranges.len() > RANGES
            || RANGES == 0
        {
            return Err(Error::InvalidConfiguration)
        }
        let mut stored = [None; RANGES];
        for (slot, range) in stored.iter_mut().zip(ranges) {
            if range.start % PAGE_SIZE != 0
                || range.length % PAGE_SIZE != 0
                || range.length == 0
                || range.start.checked_add(range.length).is_none()
            {
                return Err(Error::InvalidConfiguration)
            }
            *slot = Some(*range)
        }
        Ok(Self {
            node,
            platform,
            measurement,
            ranges: stored,
            range_count: ranges.len(),
            admitted: false,
        })
    }

    pub fn ranges(&self) -> impl Iterator<Item = AddressRange> + '_ {
        self.ranges[..self.range_count].iter().flatten().copied()
    }

    pub const fn is_admitted(&self) -> bool {
        self.admitted
    }

    pub fn protects(self, range: AddressRange) -> bool {
        let Some(range_end) = range.start.checked_add(range.length) else {
            return false
        };
        self.ranges()
            .any(|protected| protected.start <= range.start && protected.end() >= range_end)
    }

    fn attestation_digest(&self) -> [u8; 32] {
        *synos_system_model::ContentId::hash(&self.measurement).as_bytes()
    }
}

pub struct EnclaveManager<
    const ENCLAVES: usize = DEFAULT_ENCLAVE_CAPACITY,
    const RANGES: usize = DEFAULT_MEMORY_RANGE_CAPACITY,
    const CAPABILITIES: usize = 128,
> {
    admission: AdmissionController<ENCLAVES>,
    bindings: [Option<EnclaveBinding<RANGES>>; ENCLAVES],
    capabilities: CapabilityAuthority<CAPABILITIES>,
}

impl<const ENCLAVES: usize, const RANGES: usize, const CAPABILITIES: usize>
    EnclaveManager<ENCLAVES, RANGES, CAPABILITIES>
{
    pub fn new(issuer: u64) -> Result<Self, Error> {
        if ENCLAVES == 0 || RANGES == 0 || CAPABILITIES == 0 {
            return Err(Error::InvalidConfiguration)
        }
        let capabilities = CapabilityAuthority::new(issuer).ok_or(Error::InvalidConfiguration)?;
        Ok(Self {
            admission: AdmissionController::new(),
            bindings: [None; ENCLAVES],
            capabilities,
        })
    }

    pub fn register(
        &mut self,
        node: NodeId,
        platform: EnclavePlatform,
        measurement: [u8; 32],
        key: AttestationKey,
        ranges: &[AddressRange],
    ) -> Result<(), Error> {
        if self.bindings.iter().flatten().any(|binding| binding.node == node) {
            return Err(Error::InvalidConfiguration)
        }
        let binding = EnclaveBinding::new(node, platform, measurement, ranges)?;
        let slot = self
            .bindings
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(Error::Capacity)?;
        self.admission
            .register(node, platform.hardware_root(), measurement, key)?;
        *slot = Some(binding);
        Ok(())
    }

    pub fn issue_challenge(
        &mut self,
        node: NodeId,
        nonce: [u8; 32],
        expires_at_us: u64,
    ) -> Result<(), Error> {
        self.admission.issue_challenge(node, nonce, expires_at_us)?;
        if let Some(binding) = self.binding_mut(node) {
            binding.admitted = false
        }
        Ok(())
    }

    pub fn admit(&mut self, quote: AttestationQuote, now_us: u64) -> Result<(), Error> {
        self.admission.admit(quote, now_us)?;
        let binding = self.binding_mut(quote.node).ok_or(Error::NotFound)?;
        binding.admitted = true;
        audit_event!(
            Level::Info,
            EventField::unsigned(field::CALLER, quote.node.raw() as u64),
            EventField::unsigned(field::AUTH_ACTION, quote.root as u64),
        );
        Ok(())
    }

    pub fn is_admitted(&self, node: NodeId) -> bool {
        self.admission.is_admitted(node)
    }

    pub fn binding(&self, node: NodeId) -> Option<EnclaveBinding<RANGES>> {
        self.bindings.iter().flatten().find(|binding| binding.node == node).copied()
    }

    pub fn provision(
        &mut self,
        node: NodeId,
        subject: u64,
        resource: ProtectedResource,
        rights: CapabilityRights,
        expires_at_us: u64,
        now_us: u64,
    ) -> Result<ConfidentialCapability, Error> {
        let binding = self.binding(node).ok_or(Error::NotFound)?;
        self.admission.require_admitted(node)?;
        if let ProtectedResource::SharedDsm(range) = resource {
            if !binding.protects(range) {
                return Err(Error::NotProtected)
            }
        }
        self.capabilities.issue(
            node,
            subject,
            resource,
            rights,
            expires_at_us,
            now_us,
            binding.attestation_digest(),
        )
    }

    pub fn provision_dsm(
        &mut self,
        node: NodeId,
        subject: u64,
        range: AddressRange,
        rights: CapabilityRights,
        expires_at_us: u64,
        now_us: u64,
    ) -> Result<ConfidentialCapability, Error> {
        self.provision(
            node,
            subject,
            ProtectedResource::SharedDsm(range),
            rights,
            expires_at_us,
            now_us,
        )
    }

    pub fn provision_ipc(
        &mut self,
        node: NodeId,
        subject: u64,
        descriptor: InheritableDescriptor,
        rights: CapabilityRights,
        expires_at_us: u64,
        now_us: u64,
    ) -> Result<ConfidentialCapability, Error> {
        self.provision(
            node,
            subject,
            ProtectedResource::Ipc(descriptor),
            rights,
            expires_at_us,
            now_us,
        )
    }

    pub fn validate(
        &self,
        capability: ConfidentialCapability,
        subject: u64,
        required: CapabilityRights,
        now_us: u64,
    ) -> Result<(), Error> {
        self.capabilities.validate(capability, subject, required, now_us)
    }

    pub fn revoke_all(&mut self) -> u64 {
        self.capabilities.revoke_all()
    }

    fn binding_mut(&mut self, node: NodeId) -> Option<&mut EnclaveBinding<RANGES>> {
        self.bindings.iter_mut().flatten().find(|binding| binding.node == node)
    }
}

impl<const ENCLAVES: usize, const RANGES: usize, const CAPABILITIES: usize> Default
    for EnclaveManager<ENCLAVES, RANGES, CAPABILITIES>
{
    fn default() -> Self {
        Self::new(1).expect("valid default enclave manager")
    }
}
