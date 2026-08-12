//! Bounded CPU bring-up and per-CPU interrupt state shared by architecture
//! backends. Hardware code owns the actual IPI registers; this module owns
//! the lifecycle rules and never assumes an unbounded CPU count.

use crate::task::{CpuId, CpuMask, MAX_CPUS};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CpuStartupState {
    Absent,
    Requested,
    Started,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpuRecord {
    pub id: CpuId,
    pub hardware_id: u32,
    pub state: CpuStartupState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CpuTopologyError {
    InvalidCpu,
    DuplicateHardwareId,
    Capacity,
    InvalidTransition,
}

/// Fixed-capacity CPU topology used by boot and architecture backends.
pub struct CpuTopology<const CAPACITY: usize = MAX_CPUS> {
    records: [CpuRecord; CAPACITY],
    count: usize,
}

impl<const CAPACITY: usize> CpuTopology<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            records: [CpuRecord {
                id: CpuId::new(0).expect("CPU 0 is valid"),
                hardware_id: 0,
                state: CpuStartupState::Absent,
            }; CAPACITY],
            count: 0,
        }
    }

    pub fn add_bootstrap(&mut self, hardware_id: u32) -> Result<CpuId, CpuTopologyError> {
        self.add(hardware_id, CpuStartupState::Started)
    }

    pub fn request(&mut self, hardware_id: u32) -> Result<CpuId, CpuTopologyError> {
        let id = self
            .record_by_hardware_id(hardware_id)
            .map(|record| record.id)
            .unwrap_or(self.add(hardware_id, CpuStartupState::Absent)?);
        let record = self.record_mut(id)?;
        if record.state != CpuStartupState::Absent {
            return Err(CpuTopologyError::InvalidTransition)
        }
        record.state = CpuStartupState::Requested;
        Ok(id)
    }

    pub fn mark_started(&mut self, id: CpuId) -> Result<(), CpuTopologyError> {
        let record = self.record_mut(id)?;
        if !matches!(record.state, CpuStartupState::Requested | CpuStartupState::Started) {
            return Err(CpuTopologyError::InvalidTransition)
        }
        record.state = CpuStartupState::Started;
        Ok(())
    }

    pub fn mark_failed(&mut self, id: CpuId) -> Result<(), CpuTopologyError> {
        let record = self.record_mut(id)?;
        if record.state != CpuStartupState::Requested {
            return Err(CpuTopologyError::InvalidTransition)
        }
        record.state = CpuStartupState::Failed;
        Ok(())
    }

    pub fn record(&self, id: CpuId) -> Result<CpuRecord, CpuTopologyError> {
        self.records
            .get(id.raw() as usize)
            .copied()
            .filter(|record| record.state != CpuStartupState::Absent)
            .ok_or(CpuTopologyError::InvalidCpu)
    }

    pub fn online(&self) -> CpuMask {
        self.records
            .iter()
            .filter(|record| record.state == CpuStartupState::Started)
            .filter_map(|record| Some(record.id))
            .fold(CpuMask::EMPTY, |mask, cpu| {
                mask.union(CpuMask::from_cpu(cpu))
            })
    }

    pub fn online_count(&self) -> usize {
        self.records
            .iter()
            .filter(|record| record.state == CpuStartupState::Started)
            .count()
    }

    pub fn records(&self) -> impl Iterator<Item = CpuRecord> + '_ {
        self.records
            .iter()
            .copied()
            .filter(|record| record.state != CpuStartupState::Absent)
    }

    fn add(
        &mut self,
        hardware_id: u32,
        state: CpuStartupState,
    ) -> Result<CpuId, CpuTopologyError> {
        if self.records().any(|record| record.hardware_id == hardware_id) {
            return Err(CpuTopologyError::DuplicateHardwareId)
        }
        if self.count >= CAPACITY || self.count >= MAX_CPUS {
            return Err(CpuTopologyError::Capacity)
        }
        let id = CpuId::new(self.count as u8).ok_or(CpuTopologyError::Capacity)?;
        self.records[self.count] = CpuRecord {
            id,
            hardware_id,
            state,
        };
        self.count += 1;
        Ok(id)
    }

    fn record_by_hardware_id(&self, hardware_id: u32) -> Option<CpuRecord> {
        self.records()
            .find(|record| record.hardware_id == hardware_id)
    }

    fn record_mut(&mut self, id: CpuId) -> Result<&mut CpuRecord, CpuTopologyError> {
        self.records
            .get_mut(id.raw() as usize)
            .filter(|record| record.state != CpuStartupState::Absent)
            .ok_or(CpuTopologyError::InvalidCpu)
    }
}

impl<const CAPACITY: usize> Default for CpuTopology<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PerCpuInterruptState {
    pub cpu: CpuId,
    pub interrupt_depth: u16,
    pub enabled: bool,
    pub isolated: bool,
    pending_ipi: CpuMask,
}

impl PerCpuInterruptState {
    pub const fn new(cpu: CpuId) -> Self {
        Self {
            cpu,
            interrupt_depth: 0,
            enabled: false,
            isolated: false,
            pending_ipi: CpuMask::EMPTY,
        }
    }

    pub fn enter(&mut self) {
        self.interrupt_depth = self.interrupt_depth.saturating_add(1);
        self.enabled = false;
    }

    pub fn exit(&mut self) {
        self.interrupt_depth = self.interrupt_depth.saturating_sub(1);
        self.enabled = self.interrupt_depth == 0;
    }

    pub fn queue_ipi(&mut self, source: CpuId) {
        self.pending_ipi = self.pending_ipi.union(CpuMask::from_cpu(source));
    }

    pub fn take_ipi(&mut self, source: CpuId) -> bool {
        if !self.pending_ipi.contains(source) {
            return false
        }
        self.pending_ipi = self.pending_ipi.difference(CpuMask::from_cpu(source));
        true
    }
}
