use core::fmt;

use crate::task::{AddressSpaceId, CpuId};

/// Stable identifiers for the kernel invariants published in
/// `docs/invariants.toml`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum InvariantId {
    AddressSpaceOwnership = 1,
    CapabilityDerivation = 2,
    IpcOwnership = 3,
    SchedulerState = 4,
    InterruptDelivery = 5,
    PageTableTransition = 6,
}

impl InvariantId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AddressSpaceOwnership => "address_space.ownership",
            Self::CapabilityDerivation => "capability.derivation",
            Self::IpcOwnership => "ipc.ownership",
            Self::SchedulerState => "scheduler.state",
            Self::InterruptDelivery => "interrupt.delivery",
            Self::PageTableTransition => "page_table.transition",
        }
    }
}

/// The public, machine-readable description of one invariant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvariantDefinition {
    pub id: &'static str,
    pub domain: &'static str,
    pub statement: &'static str,
    pub failure_code: u16,
    pub redaction: &'static str,
}

/// The catalogue is deliberately fixed-size and contains no runtime state.
pub const CATALOGUE: &[InvariantDefinition] = &[
    InvariantDefinition {
        id: "address_space.ownership",
        domain: "address_spaces",
        statement: "A capability owner and every address-space object name a valid address space.",
        failure_code: 1001,
        redaction: "identifier-only",
    },
    InvariantDefinition {
        id: "capability.derivation",
        domain: "capabilities",
        statement: "Live capability generations, rights, backing ranges, and derivation links are consistent.",
        failure_code: 1002,
        redaction: "identifier-only",
    },
    InvariantDefinition {
        id: "ipc.ownership",
        domain: "ipc",
        statement: "Only an owner of a channel capability with the required right may use its bounded queue.",
        failure_code: 1003,
        redaction: "identifier-only",
    },
    InvariantDefinition {
        id: "scheduler.state",
        domain: "scheduler",
        statement: "Live threads have valid generations and contexts, and only one live thread runs at once.",
        failure_code: 1004,
        redaction: "identifier-only",
    },
    InvariantDefinition {
        id: "interrupt.delivery",
        domain: "interrupts",
        statement: "Interrupt vectors are bounded and isolated CPUs do not deliver kernel work.",
        failure_code: 1005,
        redaction: "identifier-only",
    },
    InvariantDefinition {
        id: "page_table.transition",
        domain: "page_tables",
        statement: "A page-table root transition uses distinct aligned frames and checked aliases.",
        failure_code: 1006,
        redaction: "identifier-only",
    },
];

/// A failure contains no handles, addresses, payloads, or tenant data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvariantFailure {
    pub invariant: InvariantId,
    pub code: u16,
}

impl InvariantFailure {
    pub const fn new(invariant: InvariantId) -> Self {
        Self {
            invariant,
            code: match invariant {
                InvariantId::AddressSpaceOwnership => 1001,
                InvariantId::CapabilityDerivation => 1002,
                InvariantId::IpcOwnership => 1003,
                InvariantId::SchedulerState => 1004,
                InvariantId::InterruptDelivery => 1005,
                InvariantId::PageTableTransition => 1006,
            },
        }
    }

    pub const fn identifier(self) -> &'static str {
        self.invariant.as_str()
    }
}

impl fmt::Display for InvariantFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invariant={} code={}", self.identifier(), self.code)
    }
}

/// Debug builds stop at the violated invariant with a redacted, stable reason.
/// Release builds keep the call sites but do not turn a diagnostic into a
/// production panic path.
#[inline]
pub fn debug_assert_valid(_result: Result<(), InvariantFailure>) {
    #[cfg(debug_assertions)]
    if let Err(failure) = _result {
        panic!("{failure}")
    }
}

pub fn check_address_space(id: AddressSpaceId) -> Result<(), InvariantFailure> {
    if id == AddressSpaceId::KERNEL || id.raw() != 0 {
        Ok(())
    } else {
        Err(InvariantFailure::new(InvariantId::AddressSpaceOwnership))
    }
}

pub const fn check_interrupt_delivery(
    vector: u64,
    cpu: CpuId,
    isolated: bool,
    delivered_to_kernel: bool,
) -> Result<(), InvariantFailure> {
    if vector >= 256 || (isolated && delivered_to_kernel) {
        return Err(InvariantFailure::new(InvariantId::InterruptDelivery))
    }
    let _ = cpu;
    Ok(())
}

pub fn check_page_table_transition<const TABLES: usize>(
    frames: &[u64; TABLES],
    physical_offset: u64,
) -> Result<(), InvariantFailure> {
    for (index, frame) in frames.iter().enumerate() {
        if *frame == 0 || *frame % 4096 != 0 || frame.checked_add(physical_offset).is_none() {
            return Err(InvariantFailure::new(InvariantId::PageTableTransition))
        }
        if frames[..index].contains(frame) {
            return Err(InvariantFailure::new(InvariantId::PageTableTransition))
        }
    }
    Ok(())
}
