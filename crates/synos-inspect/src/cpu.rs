use synos_fabric::NodeId;

use crate::InspectError;

pub const MAX_CPU_NODES: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpuNodeSample {
    pub node: NodeId,
    pub logical_cpus: u16,
    pub sample_period_us: u64,
    pub microkernel_us: u64,
    pub user_daemon_us: u64,
    pub dsm_fault_us: u64,
    pub idle_us: u64,
    pub context_switches: u64,
    pub dsm_faults: u64,
}

impl CpuNodeSample {
    pub const fn capacity_us(self) -> u64 {
        self.sample_period_us.saturating_mul(self.logical_cpus as u64)
    }
}

#[derive(Clone, Copy)]
pub struct CpuReport {
    sampled_at_us: u64,
    nodes: [Option<CpuNodeSample>; MAX_CPU_NODES],
}

impl CpuReport {
    pub const fn new() -> Self {
        Self {
            sampled_at_us: 0,
            nodes: [None; MAX_CPU_NODES],
        }
    }

    pub const fn sampled_at_us(&self) -> u64 {
        self.sampled_at_us
    }

    pub fn set_sampled_at_us(&mut self, sampled_at_us: u64) {
        self.sampled_at_us = sampled_at_us
    }

    pub fn nodes(&self) -> impl Iterator<Item = CpuNodeSample> + '_ {
        self.nodes.iter().flatten().copied()
    }

    pub fn push_node(&mut self, sample: CpuNodeSample) -> Result<(), InspectError> {
        let accounted = sample
            .microkernel_us
            .saturating_add(sample.user_daemon_us)
            .saturating_add(sample.dsm_fault_us)
            .saturating_add(sample.idle_us);
        if sample.logical_cpus == 0
            || sample.sample_period_us == 0
            || accounted > sample.capacity_us()
            || self.nodes().any(|entry| entry.node == sample.node)
        {
            return Err(InspectError::InvalidSample)
        }
        let slot = self
            .nodes
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(InspectError::Capacity)?;
        *slot = Some(sample);
        Ok(())
    }

    pub fn clear(&mut self) {
        *self = Self::new()
    }

    pub(crate) fn retain_node(&mut self, node: NodeId) {
        for entry in &mut self.nodes {
            if entry.is_some_and(|sample| sample.node != node) {
                *entry = None
            }
        }
    }
}

impl Default for CpuReport {
    fn default() -> Self {
        Self::new()
    }
}

/// Interval accountant fed by scheduler and DSM fault hooks.
pub struct CpuAccountant {
    node: NodeId,
    logical_cpus: u16,
    started_at_us: u64,
    microkernel_us: u64,
    user_daemon_us: u64,
    dsm_fault_us: u64,
    idle_us: u64,
    context_switches: u64,
    dsm_faults: u64,
}

impl CpuAccountant {
    pub const fn new(node: NodeId, logical_cpus: u16, started_at_us: u64) -> Option<Self> {
        if logical_cpus == 0 {
            return None
        }
        Some(Self {
            node,
            logical_cpus,
            started_at_us,
            microkernel_us: 0,
            user_daemon_us: 0,
            dsm_fault_us: 0,
            idle_us: 0,
            context_switches: 0,
            dsm_faults: 0,
        })
    }

    pub fn record_microkernel(&mut self, duration_us: u64) {
        self.microkernel_us = self.microkernel_us.saturating_add(duration_us)
    }

    pub fn record_user_daemon(&mut self, duration_us: u64) {
        self.user_daemon_us = self.user_daemon_us.saturating_add(duration_us)
    }

    pub fn record_dsm_fault(&mut self, duration_us: u64) {
        self.dsm_fault_us = self.dsm_fault_us.saturating_add(duration_us);
        self.dsm_faults = self.dsm_faults.saturating_add(1)
    }

    pub fn record_idle(&mut self, duration_us: u64) {
        self.idle_us = self.idle_us.saturating_add(duration_us)
    }

    pub fn record_context_switch(&mut self) {
        self.context_switches = self.context_switches.saturating_add(1)
    }

    pub fn finish(&mut self, now_us: u64) -> Result<CpuNodeSample, InspectError> {
        let sample = CpuNodeSample {
            node: self.node,
            logical_cpus: self.logical_cpus,
            sample_period_us: now_us
                .checked_sub(self.started_at_us)
                .filter(|period| *period != 0)
                .ok_or(InspectError::InvalidSample)?,
            microkernel_us: self.microkernel_us,
            user_daemon_us: self.user_daemon_us,
            dsm_fault_us: self.dsm_fault_us,
            idle_us: self.idle_us,
            context_switches: self.context_switches,
            dsm_faults: self.dsm_faults,
        };
        let mut validation = CpuReport::new();
        validation.push_node(sample)?;
        self.started_at_us = now_us;
        self.microkernel_us = 0;
        self.user_daemon_us = 0;
        self.dsm_fault_us = 0;
        self.idle_us = 0;
        self.context_switches = 0;
        self.dsm_faults = 0;
        Ok(sample)
    }
}
